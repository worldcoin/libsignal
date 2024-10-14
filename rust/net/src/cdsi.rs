//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use std::default::Default;

use http::StatusCode;
use libsignal_core::{Aci, Pni, E164};
use libsignal_net_infra::connection_manager::ConnectionManager;
use libsignal_net_infra::errors::TransportConnectError;
use libsignal_net_infra::ws::{
    AttestedConnection, AttestedConnectionError, NextOrClose, WebSocketConnectError,
    WebSocketServiceError,
};
use libsignal_net_infra::{
    extract_retry_after_seconds, AsyncDuplexStream, HttpBasicAuth, TransportConnector,
};
use prost::Message as _;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_boring_signal::SslStream;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::CloseFrame;
use uuid::Uuid;

use crate::enclave::{Cdsi, EnclaveEndpointConnection};
use crate::proto::cds2::{ClientRequest, ClientResponse};

trait FixedLengthSerializable {
    const SERIALIZED_LEN: usize;

    // TODO: when feature(generic_const_exprs) is stabilized, make the target an
    // array reference instead of a slice.
    fn serialize_into(&self, target: &mut [u8]);
}

trait CollectSerialized {
    fn collect_serialized(self) -> Vec<u8>;
}

impl<It: ExactSizeIterator<Item = T>, T: FixedLengthSerializable> CollectSerialized for It {
    fn collect_serialized(self) -> Vec<u8> {
        let mut output = vec![0; T::SERIALIZED_LEN * self.len()];
        for (item, chunk) in self.zip(output.chunks_mut(T::SERIALIZED_LEN)) {
            item.serialize_into(chunk)
        }

        output
    }
}

impl FixedLengthSerializable for E164 {
    const SERIALIZED_LEN: usize = 8;

    fn serialize_into(&self, target: &mut [u8]) {
        target.copy_from_slice(&self.to_be_bytes())
    }
}

impl FixedLengthSerializable for Uuid {
    const SERIALIZED_LEN: usize = 16;
    fn serialize_into(&self, target: &mut [u8]) {
        target.copy_from_slice(self.as_bytes())
    }
}

pub struct AciAndAccessKey {
    pub aci: Aci,
    pub access_key: [u8; 16],
}

impl FixedLengthSerializable for AciAndAccessKey {
    const SERIALIZED_LEN: usize = 32;

    fn serialize_into(&self, target: &mut [u8]) {
        let (aci_bytes, access_key_bytes) = target.split_at_mut(Uuid::SERIALIZED_LEN);

        Uuid::from(self.aci).serialize_into(aci_bytes);
        access_key_bytes.copy_from_slice(&self.access_key)
    }
}

#[derive(Default)]
pub struct LookupRequest {
    pub new_e164s: Vec<E164>,
    pub prev_e164s: Vec<E164>,
    pub acis_and_access_keys: Vec<AciAndAccessKey>,
    pub return_acis_without_uaks: bool,
    pub token: Box<[u8]>,
}

impl LookupRequest {
    fn into_client_request(self) -> ClientRequest {
        let Self {
            new_e164s,
            prev_e164s,
            acis_and_access_keys,
            return_acis_without_uaks,
            token,
        } = self;

        let aci_uak_pairs = acis_and_access_keys.into_iter().collect_serialized();
        let new_e164s = new_e164s.into_iter().collect_serialized();
        let prev_e164s = prev_e164s.into_iter().collect_serialized();

        ClientRequest {
            aci_uak_pairs,
            new_e164s,
            prev_e164s,
            return_acis_without_uaks,
            token: token.into_vec(),
            token_ack: false,
            // TODO: use these for supporting non-desktop client requirements.
            discard_e164s: Vec::new(),
        }
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Token(pub Box<[u8]>);

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct LookupResponse {
    pub records: Vec<LookupResponseEntry>,
    pub debug_permits_used: i32,
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct LookupResponseEntry {
    pub e164: E164,
    pub aci: Option<Aci>,
    pub pni: Option<Pni>,
}

#[derive(Debug, PartialEq)]
pub enum LookupResponseParseError {
    InvalidNumberOfBytes { actual_length: usize },
}

impl From<LookupResponseParseError> for LookupError {
    fn from(value: LookupResponseParseError) -> Self {
        match value {
            LookupResponseParseError::InvalidNumberOfBytes { .. } => Self::ParseError,
        }
    }
}

impl TryFrom<ClientResponse> for LookupResponse {
    type Error = LookupResponseParseError;

    fn try_from(response: ClientResponse) -> Result<Self, Self::Error> {
        let ClientResponse {
            e164_pni_aci_triples,
            token: _,
            debug_permits_used,
        } = response;

        if e164_pni_aci_triples.len() % LookupResponseEntry::SERIALIZED_LEN != 0 {
            return Err(LookupResponseParseError::InvalidNumberOfBytes {
                actual_length: e164_pni_aci_triples.len(),
            });
        }

        let records = e164_pni_aci_triples
            .chunks(LookupResponseEntry::SERIALIZED_LEN)
            .flat_map(|record| {
                LookupResponseEntry::try_parse_from(
                    record.try_into().expect("chunk size is correct"),
                )
            })
            .collect();

        Ok(Self {
            records,
            debug_permits_used,
        })
    }
}

impl LookupResponseEntry {
    fn try_parse_from(record: &[u8; Self::SERIALIZED_LEN]) -> Option<Self> {
        fn non_nil_uuid<T: From<Uuid>>(bytes: &uuid::Bytes) -> Option<T> {
            let uuid = Uuid::from_bytes(*bytes);
            (!uuid.is_nil()).then(|| uuid.into())
        }

        // TODO(https://github.com/rust-lang/rust/issues/90091): use split_array
        // instead of expect() on the output.
        let (e164_bytes, record) = record.split_at(E164::SERIALIZED_LEN);
        let e164_bytes = <&[u8; E164::SERIALIZED_LEN]>::try_from(e164_bytes).expect("split at len");
        let e164 = E164::from_be_bytes(*e164_bytes)?;
        let (pni_bytes, aci_bytes) = record.split_at(Uuid::SERIALIZED_LEN);

        let pni = non_nil_uuid(pni_bytes.try_into().expect("split at len"));
        let aci = non_nil_uuid(aci_bytes.try_into().expect("split at len"));

        Some(Self { e164, aci, pni })
    }
}

impl FixedLengthSerializable for LookupResponseEntry {
    const SERIALIZED_LEN: usize = E164::SERIALIZED_LEN + Uuid::SERIALIZED_LEN * 2;

    fn serialize_into(&self, target: &mut [u8]) {
        let Self { e164, aci, pni } = self;

        let (e164_bytes, target) = target.split_at_mut(E164::SERIALIZED_LEN);
        e164.serialize_into(e164_bytes);

        let (pni_bytes, aci_bytes) = target.split_at_mut(Uuid::SERIALIZED_LEN);
        pni.map(Uuid::from)
            .unwrap_or(Uuid::nil())
            .serialize_into(pni_bytes);

        aci.map(Uuid::from)
            .unwrap_or(Uuid::nil())
            .serialize_into(aci_bytes);
    }
}

#[cfg_attr(test, derive(Debug))]
pub struct CdsiConnection<S>(AttestedConnection<S>);

impl<S> AsMut<AttestedConnection<S>> for CdsiConnection<S> {
    fn as_mut(&mut self) -> &mut AttestedConnection<S> {
        &mut self.0
    }
}

/// Anything that can go wrong during a CDSI lookup.
#[derive(Debug, Error, displaydoc::Display)]
pub enum LookupError {
    /// protocol error after establishing a connection
    Protocol,
    /// SGX attestation failed.
    AttestationError(attest::enclave::Error),
    /// invalid response received from the server
    InvalidResponse,
    /// retry later
    RateLimited { retry_after_seconds: u32 },
    /// request token was invalid
    InvalidToken,
    /// failed to parse the response from the server
    ParseError,
    /// transport failed: {0}
    ConnectTransport(TransportConnectError),
    /// websocket error: {0}
    WebSocket(WebSocketServiceError),
    /// connect attempt timed out
    ConnectionTimedOut,
    /// request was invalid: {server_reason}
    InvalidArgument { server_reason: String },
    /// server error: {reason}
    Server { reason: &'static str },
}

impl From<AttestedConnectionError> for LookupError {
    fn from(value: AttestedConnectionError) -> Self {
        match value {
            AttestedConnectionError::ClientConnection(_) => Self::Protocol,
            AttestedConnectionError::WebSocket(e) => Self::WebSocket(e),
            AttestedConnectionError::Protocol => Self::Protocol,
            AttestedConnectionError::Attestation(e) => Self::AttestationError(e),
        }
    }
}

impl From<crate::enclave::Error> for LookupError {
    fn from(value: crate::enclave::Error) -> Self {
        use crate::enclave::Error;
        match value {
            Error::WebSocketConnect(err) => match err {
                WebSocketConnectError::Timeout => Self::ConnectionTimedOut,
                WebSocketConnectError::Transport(e) => Self::ConnectTransport(e),
                WebSocketConnectError::RejectedByServer {
                    response,
                    received_at: _,
                } => {
                    if response.status() == StatusCode::TOO_MANY_REQUESTS {
                        if let Some(retry_after_seconds) =
                            extract_retry_after_seconds(response.headers())
                        {
                            return Self::RateLimited {
                                retry_after_seconds,
                            };
                        }
                    }
                    Self::WebSocket(WebSocketServiceError::Http(response))
                }
                WebSocketConnectError::WebSocketError(e) => Self::WebSocket(e.into()),
            },
            Error::AttestationError(err) => Self::AttestationError(err),
            Error::WebSocket(err) => Self::WebSocket(err),
            Error::Protocol => Self::Protocol,
            Error::ConnectionTimedOut => Self::ConnectionTimedOut,
        }
    }
}

impl From<prost::DecodeError> for LookupError {
    fn from(_value: prost::DecodeError) -> Self {
        Self::Protocol
    }
}

#[derive(serde::Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
struct RateLimitExceededResponse {
    retry_after_seconds: u32,
}

#[cfg_attr(test, derive(Debug))]
pub struct ClientResponseCollector<S = SslStream<TcpStream>>(CdsiConnection<S>);

impl<S: AsyncDuplexStream> CdsiConnection<S> {
    /// Connect to remote host and verify remote attestation.
    pub async fn connect<C, T>(
        endpoint: &EnclaveEndpointConnection<Cdsi, C>,
        transport_connector: T,
        auth: impl HttpBasicAuth,
    ) -> Result<Self, LookupError>
    where
        C: ConnectionManager,
        T: TransportConnector<Stream = S>,
    {
        let connection = endpoint.connect(auth, transport_connector).await?;
        Ok(Self(connection))
    }

    pub async fn send_request(
        mut self,
        request: LookupRequest,
    ) -> Result<(Token, ClientResponseCollector<S>), LookupError> {
        self.0.send(request.into_client_request()).await?;
        let token_response: ClientResponse = self.0.receive().await?.next_or_else(|close| {
            close
                .and_then(err_for_close)
                .unwrap_or(LookupError::Protocol)
        })?;

        if token_response.token.is_empty() {
            return Err(LookupError::Protocol);
        }

        Ok((
            Token(token_response.token.into_boxed_slice()),
            ClientResponseCollector(self),
        ))
    }
}

impl<S: AsyncDuplexStream> ClientResponseCollector<S> {
    pub async fn collect(self) -> Result<LookupResponse, LookupError> {
        let Self(mut connection) = self;

        let token_ack = ClientRequest {
            token_ack: true,
            ..Default::default()
        };

        connection.0.send(token_ack).await?;
        let mut response: ClientResponse = connection.0.receive().await?.next_or_else(|close| {
            close
                .and_then(err_for_close)
                .unwrap_or(LookupError::Protocol)
        })?;
        loop {
            match connection.0.receive_bytes().await? {
                NextOrClose::Next(decoded) => {
                    response
                        .merge(decoded.as_ref())
                        .map_err(LookupError::from)?;
                }
                NextOrClose::Close(
                    None
                    | Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: _,
                    }),
                ) => break,
                NextOrClose::Close(Some(close)) => {
                    return Err(err_for_close(close).unwrap_or(LookupError::Protocol))
                }
            }
        }
        Ok(response.try_into()?)
    }
}

/// Numeric code set by the server on the websocket close frame.
#[repr(u16)]
#[derive(Copy, Clone, num_enum::TryFromPrimitive, strum::IntoStaticStr)]
enum CdsiCloseCode {
    InvalidArgument = 4003,
    RateLimitExceeded = 4008,
    ServerInternalError = 4013,
    ServerUnavailable = 4014,
    InvalidToken = 4101,
}

/// Produces a [`LookupError`] for the provided [`CloseFrame`].
///
/// Returns `Some(err)` if there is a relevant `LookupError` value for the
/// provided close frame. Otherwise returns `None`.
fn err_for_close(CloseFrame { code, reason }: CloseFrame<'_>) -> Option<LookupError> {
    let Ok(code) = CdsiCloseCode::try_from(u16::from(code)) else {
        log::warn!("got unexpected websocket error code: {code}",);
        return None;
    };

    match code {
        CdsiCloseCode::InvalidArgument => Some(LookupError::InvalidArgument {
            server_reason: reason.into_owned(),
        }),
        CdsiCloseCode::InvalidToken => Some(LookupError::InvalidToken),
        CdsiCloseCode::RateLimitExceeded => {
            let RateLimitExceededResponse {
                retry_after_seconds,
            } = serde_json::from_str(&reason).ok()?;
            Some(LookupError::RateLimited {
                retry_after_seconds,
            })
        }
        CdsiCloseCode::ServerInternalError | CdsiCloseCode::ServerUnavailable => {
            Some(LookupError::Server {
                reason: code.into(),
            })
        }
    }
}

#[cfg(test)]
mod test {
    use std::num::NonZeroU64;
    use std::time::Duration;

    use assert_matches::assert_matches;
    use hex_literal::hex;
    use libsignal_net_infra::testutil::InMemoryWarpConnector;
    use libsignal_net_infra::utils::ObservableEvent;
    use libsignal_net_infra::ws::testutil::{
        fake_websocket, mock_connection_info, run_attested_server, AttestedServerOutput,
        FAKE_ATTESTATION,
    };
    use libsignal_net_infra::ws::WebSocketClient;
    use nonzero_ext::nonzero;
    use tungstenite::protocol::frame::coding::CloseCode;
    use tungstenite::protocol::CloseFrame;
    use uuid::Uuid;
    use warp::Filter as _;

    use super::*;
    use crate::auth::Auth;

    #[test]
    fn parse_lookup_response_entries() {
        const ACI_BYTES: [u8; 16] = hex!("0102030405060708a1a2a3a4a5a6a7a8");
        const PNI_BYTES: [u8; 16] = hex!("b1b2b3b4b5b6b7b81112131415161718");

        let e164: E164 = "+18005551001".parse().unwrap();
        let mut e164_bytes = [0; 8];
        e164.serialize_into(&mut e164_bytes);

        // Generate a sequence of triples by repeating the above data a few times.
        const NUM_REPEATS: usize = 4;
        let e164_pni_aci_triples =
            std::iter::repeat([e164_bytes.as_slice(), &PNI_BYTES, &ACI_BYTES])
                .take(NUM_REPEATS)
                .flatten()
                .flatten()
                .cloned()
                .collect();

        let parsed = ClientResponse {
            e164_pni_aci_triples,
            token: vec![],
            debug_permits_used: 42,
        }
        .try_into();
        assert_eq!(
            parsed,
            Ok(LookupResponse {
                records: vec![
                    LookupResponseEntry {
                        e164,
                        aci: Some(Aci::from(Uuid::from_bytes(ACI_BYTES))),
                        pni: Some(Pni::from(Uuid::from_bytes(PNI_BYTES))),
                    };
                    NUM_REPEATS
                ],
                debug_permits_used: 42
            })
        );
    }

    #[test]
    fn serialize_e164s() {
        let e164s: Vec<E164> = (18005551001..)
            .take(5)
            .map(|n| E164::new(NonZeroU64::new(n).unwrap()))
            .collect();
        let serialized = e164s.into_iter().collect_serialized();

        assert_eq!(
            serialized.as_slice(),
            &hex!(
                "000000043136e799"
                "000000043136e79a"
                "000000043136e79b"
                "000000043136e79c"
                "000000043136e79d"
            )
        );
    }

    #[test]
    fn serialize_acis_and_access_keys() {
        let pairs = [1, 2, 3, 4, 5].map(|i| AciAndAccessKey {
            access_key: [i; 16],
            aci: Aci::from_uuid_bytes([i | 0x80; 16]),
        });
        let serialized = pairs.into_iter().collect_serialized();

        assert_eq!(
            serialized.as_slice(),
            &hex!(
                "8181818181818181818181818181818101010101010101010101010101010101"
                "8282828282828282828282828282828202020202020202020202020202020202"
                "8383838383838383838383838383838303030303030303030303030303030303"
                "8484848484848484848484848484848404040404040404040404040404040404"
                "8585858585858585858585858585858505050505050505050505050505050505"
            )
        );
    }

    /// Server-side state relative to a remote request.
    #[derive(Debug, Default, PartialEq)]
    enum FakeServerState {
        /// The client has not yet sent the first request message.
        #[default]
        AwaitingLookupRequest,
        /// Token response was sent, waiting for the client to ack it.
        AwaitingTokenAck,
        /// All response messages have been sent.
        Finished,
    }

    impl FakeServerState {
        const RESPONSE_TOKEN: &'static [u8] = b"new token";
        const RESPONSE_RECORD: LookupResponseEntry = LookupResponseEntry {
            aci: Some(Aci::from_uuid_bytes([b'a'; 16])),
            pni: Some(Pni::from_uuid_bytes([b'p'; 16])),
            e164: E164::new(nonzero!(18005550101u64)),
        };

        fn receive_frame(&mut self, frame: &[u8]) -> AttestedServerOutput {
            match self {
                Self::AwaitingLookupRequest => {
                    let _client_request = ClientRequest::decode(frame).expect("can decode");

                    *self = Self::AwaitingTokenAck;
                    AttestedServerOutput::message(
                        ClientResponse {
                            token: Self::RESPONSE_TOKEN.into(),
                            ..Default::default()
                        }
                        .encode_to_vec(),
                    )
                }
                Self::AwaitingTokenAck => {
                    let client_request = ClientRequest::decode(frame).expect("can decode");
                    assert!(
                        client_request.token_ack,
                        "invalid message: {client_request:?}"
                    );
                    *self = Self::Finished;
                    let mut triples_bytes = [0; LookupResponseEntry::SERIALIZED_LEN];
                    Self::RESPONSE_RECORD.serialize_into(&mut triples_bytes);
                    AttestedServerOutput {
                        message: Some(
                            ClientResponse {
                                debug_permits_used: 1,
                                e164_pni_aci_triples: triples_bytes.to_vec(),
                                ..Default::default()
                            }
                            .encode_to_vec(),
                        ),
                        close_after: Some(None),
                    }
                }
                Self::Finished => {
                    panic!("no frame expected");
                }
            }
        }

        /// Produces a closure usable with [`run_attested_server`].
        fn into_handler(mut self) -> impl FnMut(NextOrClose<Vec<u8>>) -> AttestedServerOutput {
            move |frame| {
                let frame = match frame {
                    NextOrClose::Close(_) => panic!("unexpected client-originating close"),
                    NextOrClose::Next(frame) => frame,
                };
                self.receive_frame(&frame)
            }
        }

        fn into_handler_with_close_from(
            mut self,
            state_before_close: &'static FakeServerState,
            close_frame: CloseFrame<'static>,
        ) -> impl FnMut(NextOrClose<Vec<u8>>) -> AttestedServerOutput {
            move |frame| {
                if &self == state_before_close {
                    return AttestedServerOutput::close(Some(close_frame.clone()));
                }

                let frame = match frame {
                    NextOrClose::Close(_) => panic!("unexpected client-originating close"),
                    NextOrClose::Next(frame) => frame,
                };
                self.receive_frame(&frame)
            }
        }
    }

    #[tokio::test]
    async fn lookup_success() {
        let (server, client) = fake_websocket().await;

        let fake_server = FakeServerState::default().into_handler();
        tokio::spawn(run_attested_server(
            server,
            attest::sgx_session::testutil::private_key(),
            fake_server,
        ));

        let ws_client = WebSocketClient::new_fake(client, mock_connection_info());
        let cdsi_connection = CdsiConnection(
            AttestedConnection::connect(ws_client, |fake_attestation| {
                assert_eq!(fake_attestation, FAKE_ATTESTATION);
                attest::sgx_session::testutil::handshake_from_tests_data()
            })
            .await
            .expect("handshake failed"),
        );

        let (token, collector) = cdsi_connection
            .send_request(LookupRequest {
                token: b"valid but ignored token".as_slice().into(),
                ..Default::default()
            })
            .await
            .expect("request accepted");

        assert_eq!(&*token.0, FakeServerState::RESPONSE_TOKEN);

        let response = collector.collect().await.expect("successful request");

        assert_eq!(
            response,
            LookupResponse {
                debug_permits_used: 1,
                records: vec![FakeServerState::RESPONSE_RECORD],
            }
        );
    }

    const RETRY_AFTER_SECS: u32 = 12345;

    #[tokio::test]
    async fn websocket_close_with_rate_limit_exceeded_after_initial_request() {
        let (server, client) = fake_websocket().await;

        let fake_server = FakeServerState::default().into_handler_with_close_from(
            &FakeServerState::AwaitingLookupRequest,
            CloseFrame {
                code: CloseCode::Bad(4008),
                reason: serde_json::to_string_pretty(&RateLimitExceededResponse {
                    retry_after_seconds: RETRY_AFTER_SECS,
                })
                .expect("can JSON-encode")
                .into(),
            },
        );

        tokio::spawn(run_attested_server(
            server,
            attest::sgx_session::testutil::private_key(),
            fake_server,
        ));

        let ws_client = WebSocketClient::new_fake(client, mock_connection_info());
        let cdsi_connection = CdsiConnection(
            AttestedConnection::connect(ws_client, |fake_attestation| {
                assert_eq!(fake_attestation, FAKE_ATTESTATION);
                attest::sgx_session::testutil::handshake_from_tests_data()
            })
            .await
            .expect("handshake failed"),
        );

        let response = cdsi_connection
            .send_request(LookupRequest {
                token: b"valid but ignored token".as_slice().into(),
                ..Default::default()
            })
            .await;

        assert_matches!(
            response,
            Err(LookupError::RateLimited {
                retry_after_seconds: RETRY_AFTER_SECS
            })
        );
    }

    #[tokio::test]
    async fn websocket_close_with_rate_limit_exceeded_after_token_ack() {
        let (server, client) = fake_websocket().await;

        let fake_server = FakeServerState::default().into_handler_with_close_from(
            &FakeServerState::AwaitingTokenAck,
            CloseFrame {
                code: CloseCode::Bad(4008),
                reason: serde_json::to_string_pretty(&RateLimitExceededResponse {
                    retry_after_seconds: RETRY_AFTER_SECS,
                })
                .expect("can JSON-encode")
                .into(),
            },
        );

        tokio::spawn(run_attested_server(
            server,
            attest::sgx_session::testutil::private_key(),
            fake_server,
        ));

        let ws_client = WebSocketClient::new_fake(client, mock_connection_info());
        let cdsi_connection = CdsiConnection(
            AttestedConnection::connect(ws_client, |fake_attestation| {
                assert_eq!(fake_attestation, FAKE_ATTESTATION);
                attest::sgx_session::testutil::handshake_from_tests_data()
            })
            .await
            .expect("handshake failed"),
        );

        let (_token, collector) = cdsi_connection
            .send_request(LookupRequest {
                token: b"valid but ignored token".as_slice().into(),
                ..Default::default()
            })
            .await
            .expect("request accepted");

        let response = collector.collect().await;

        assert_matches!(
            response,
            Err(LookupError::RateLimited {
                retry_after_seconds: RETRY_AFTER_SECS
            })
        )
    }

    #[tokio::test]
    async fn websocket_rejected_with_http_429_too_many_requests() {
        let h2_server = warp::get().then(|| async move {
            warp::reply::with_status(
                warp::reply::with_header("(ignored body)", "Retry-After", "100"),
                warp::http::StatusCode::TOO_MANY_REQUESTS,
            )
        });
        let connector = InMemoryWarpConnector::new(h2_server);

        let env = crate::env::PROD;
        let endpoint_connection = EnclaveEndpointConnection::new(
            &env.cdsi,
            Duration::from_secs(10),
            &ObservableEvent::default(),
        );
        let auth = Auth {
            username: "username".to_string(),
            password: "password".to_string(),
        };

        let result = CdsiConnection::connect(&endpoint_connection, connector, auth).await;
        assert_matches!(
            result,
            Err(LookupError::RateLimited {
                retry_after_seconds: 100
            })
        )
    }

    #[tokio::test]
    async fn websocket_invalid_token_close() {
        let (server, client) = fake_websocket().await;

        const INVALID_TOKEN: &[u8] = b"invalid token";
        let fake_server = FakeServerState::default().into_handler_with_close_from(
            &FakeServerState::AwaitingLookupRequest,
            CloseFrame {
                code: CloseCode::Bad(4101),
                reason: "invalid token".into(),
            },
        );

        tokio::spawn(run_attested_server(
            server,
            attest::sgx_session::testutil::private_key(),
            fake_server,
        ));

        let ws_client = WebSocketClient::new_fake(client, mock_connection_info());
        let cdsi_connection = CdsiConnection(
            AttestedConnection::connect(ws_client, |fake_attestation| {
                assert_eq!(fake_attestation, FAKE_ATTESTATION);
                attest::sgx_session::testutil::handshake_from_tests_data()
            })
            .await
            .expect("handshake failed"),
        );

        let response = cdsi_connection
            .send_request(LookupRequest {
                token: INVALID_TOKEN.into(),
                ..Default::default()
            })
            .await;

        assert_matches!(response, Err(LookupError::InvalidToken));
    }
}
