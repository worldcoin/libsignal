//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use std::str::FromStr;
use std::time::Duration;

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use libsignal_bridge_macros::*;
use libsignal_bridge_types::net::chat::{
    AuthChat, HttpRequest, ResponseAndDebugInfo, ServerMessageAck,
};
use libsignal_bridge_types::net::TokioAsyncContext;
use libsignal_core::E164;
use libsignal_net::cdsi::{LookupError, LookupResponse, LookupResponseEntry};
use libsignal_net::chat::{
    self, ChatServiceError, DebugInfo as ChatServiceDebugInfo, Response as ChatResponse,
};
use libsignal_net::infra::ws::WebSocketServiceError;
use libsignal_net::infra::IpType;
use libsignal_protocol::{Aci, Pni};
use nonzero_ext::nonzero;
use uuid::Uuid;

use crate::*;

#[bridge_io(TokioAsyncContext)]
async fn TESTING_CdsiLookupResponseConvert() -> LookupResponse {
    const E164_BOTH: E164 = E164::new(nonzero!(18005551011u64));
    const E164_PNI: E164 = E164::new(nonzero!(18005551012u64));
    const ACI_UUID: &str = "9d0652a3-dcc3-4d11-975f-74d61598733f";
    const PNI_UUID: &str = "796abedb-ca4e-4f18-8803-1fde5b921f9f";
    const DEBUG_PERMITS_USED: i32 = 123;

    let aci = Aci::from(Uuid::parse_str(ACI_UUID).expect("is valid"));
    let pni = Pni::from(Uuid::parse_str(PNI_UUID).expect("is valid"));

    LookupResponse {
        records: vec![
            LookupResponseEntry {
                e164: E164_BOTH,
                aci: Some(aci),
                pni: Some(pni),
            },
            LookupResponseEntry {
                e164: E164_PNI,
                pni: Some(pni),
                aci: None,
            },
        ],
        debug_permits_used: DEBUG_PERMITS_USED,
    }
}

#[bridge_io(TokioAsyncContext)]
async fn TESTING_OnlyCompletesByCancellation() {
    std::future::pending::<()>().await
}

macro_rules! make_error_testing_enum {
    (enum $name:ident for $orig:ident {
        $($orig_case:ident => $case:ident,)*
        $(; $($extra_case:ident,)*)?
    }) => {
        #[derive(Copy, Clone, strum::EnumString)]
        enum $name {
            $($case,)*
            $($($extra_case,)*)?
        }
        const _: () = {
            /// This code isn't ever executed. It exists so that when new cases are
            /// added to the original enum, this will fail to compile until corresponding
            /// cases are added to the testing enum.
            #[allow(unused)]
            fn match_on_lookup_error(value: &'static $orig) -> $name {
                match value {
                    $($orig::$orig_case { .. } => $name::$case),*
                }
            }
        };
        impl TryFrom<String> for $name {
            type Error = <Self as FromStr>::Err;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                FromStr::from_str(&value)
            }
        }
    }
}

make_error_testing_enum! {
    enum TestingCdsiLookupError for LookupError {
        Protocol => Protocol,
        AttestationError => AttestationDataError,
        InvalidResponse => InvalidResponse,
        RateLimited => RetryAfter42Seconds,
        InvalidToken => InvalidToken,
        InvalidArgument => InvalidArgument,
        ParseError => Parse,
        ConnectTransport => ConnectDnsFailed,
        WebSocket => WebSocketIdleTooLong,
        ConnectionTimedOut => ConnectionTimedOut,
        Server => ServerCrashed,
    }
}

/// Return an error matching the requested description.
#[bridge_fn]
fn TESTING_CdsiLookupErrorConvert(
    // The stringly-typed API makes the call sites more self-explanatory.
    error_description: AsType<TestingCdsiLookupError, String>,
) -> Result<(), LookupError> {
    Err(match error_description.into_inner() {
        TestingCdsiLookupError::Protocol => LookupError::Protocol,
        TestingCdsiLookupError::AttestationDataError => {
            LookupError::AttestationError(attest::enclave::Error::AttestationDataError {
                reason: "fake reason".into(),
            })
        }
        TestingCdsiLookupError::InvalidResponse => LookupError::InvalidResponse,
        TestingCdsiLookupError::RetryAfter42Seconds => LookupError::RateLimited {
            retry_after_seconds: 42,
        },
        TestingCdsiLookupError::InvalidToken => LookupError::InvalidToken,
        TestingCdsiLookupError::InvalidArgument => LookupError::InvalidArgument {
            server_reason: "fake reason".into(),
        },
        TestingCdsiLookupError::Parse => LookupError::ParseError,
        TestingCdsiLookupError::ConnectDnsFailed => LookupError::ConnectTransport(
            libsignal_net::infra::errors::TransportConnectError::DnsError,
        ),
        TestingCdsiLookupError::WebSocketIdleTooLong => LookupError::WebSocket(
            libsignal_net::infra::ws::WebSocketServiceError::ChannelIdleTooLong,
        ),
        TestingCdsiLookupError::ConnectionTimedOut => LookupError::ConnectionTimedOut,
        TestingCdsiLookupError::ServerCrashed => LookupError::Server { reason: "crashed" },
    })
}

make_error_testing_enum! {
    enum TestingChatServiceError for ChatServiceError {
        WebSocket => WebSocket,
        AppExpired => AppExpired,
        DeviceDeregistered => DeviceDeregistered,
        UnexpectedFrameReceived => UnexpectedFrameReceived,
        ServerRequestMissingId => ServerRequestMissingId,
        FailedToPassMessageToIncomingChannel => FailedToPassMessageToIncomingChannel,
        IncomingDataInvalid => IncomingDataInvalid,
        RequestHasInvalidHeader => RequestHasInvalidHeader,
        Timeout => Timeout,
        TimeoutEstablishingConnection => TimeoutEstablishingConnection,
        AllConnectionRoutesFailed => AllConnectionRoutesFailed,
        ServiceInactive => ServiceInactive,
        ServiceUnavailable => ServiceUnavailable,
        ServiceIntentionallyDisconnected => ServiceIntentionallyDisconnected,
    }
}

#[bridge_fn]
fn TESTING_ChatServiceErrorConvert(
    // The stringly-typed API makes the call sites more self-explanatory.
    error_description: AsType<TestingChatServiceError, String>,
) -> Result<(), ChatServiceError> {
    Err(match error_description.into_inner() {
        TestingChatServiceError::WebSocket => ChatServiceError::WebSocket(
            libsignal_net::infra::ws::WebSocketServiceError::Other("testing"),
        ),
        TestingChatServiceError::AppExpired => ChatServiceError::AppExpired,
        TestingChatServiceError::DeviceDeregistered => ChatServiceError::DeviceDeregistered,
        TestingChatServiceError::UnexpectedFrameReceived => {
            ChatServiceError::UnexpectedFrameReceived
        }
        TestingChatServiceError::ServerRequestMissingId => ChatServiceError::ServerRequestMissingId,
        TestingChatServiceError::FailedToPassMessageToIncomingChannel => {
            ChatServiceError::FailedToPassMessageToIncomingChannel
        }
        TestingChatServiceError::IncomingDataInvalid => ChatServiceError::IncomingDataInvalid,
        TestingChatServiceError::RequestHasInvalidHeader => {
            ChatServiceError::RequestHasInvalidHeader
        }
        TestingChatServiceError::Timeout => ChatServiceError::Timeout,
        TestingChatServiceError::TimeoutEstablishingConnection => {
            ChatServiceError::TimeoutEstablishingConnection { attempts: 42 }
        }
        TestingChatServiceError::AllConnectionRoutesFailed => {
            ChatServiceError::AllConnectionRoutesFailed { attempts: 42 }
        }
        TestingChatServiceError::ServiceInactive => ChatServiceError::ServiceInactive,
        TestingChatServiceError::ServiceUnavailable => ChatServiceError::ServiceUnavailable,
        TestingChatServiceError::ServiceIntentionallyDisconnected => {
            ChatServiceError::ServiceIntentionallyDisconnected
        }
    })
}

#[bridge_fn]
fn TESTING_ChatServiceResponseConvert(
    body_present: bool,
) -> Result<ChatResponse, ChatServiceError> {
    let body = match body_present {
        true => Some(b"content".to_vec().into_boxed_slice()),
        false => None,
    };
    let mut headers = HeaderMap::new();
    headers.append(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.append(http::header::FORWARDED, HeaderValue::from_static("1.1.1.1"));
    Ok(ChatResponse {
        status: StatusCode::OK,
        message: Some("OK".to_string()),
        body,
        headers,
    })
}

#[bridge_fn]
fn TESTING_ChatServiceDebugInfoConvert() -> Result<ChatServiceDebugInfo, ChatServiceError> {
    Ok(ChatServiceDebugInfo {
        ip_type: IpType::V4,
        duration: Duration::from_millis(200),
        connection_info: "connection_info".to_string(),
    })
}

#[bridge_fn]
fn TESTING_ChatServiceResponseAndDebugInfoConvert() -> Result<ResponseAndDebugInfo, ChatServiceError>
{
    Ok(ResponseAndDebugInfo {
        response: TESTING_ChatServiceResponseConvert(true)?,
        debug_info: TESTING_ChatServiceDebugInfoConvert()?,
    })
}

#[bridge_fn]
fn TESTING_ChatRequestGetMethod(request: &HttpRequest) -> String {
    request.method.to_string()
}

#[bridge_fn]
fn TESTING_ChatRequestGetPath(request: &HttpRequest) -> String {
    request.path.to_string()
}

#[bridge_fn]
fn TESTING_ChatRequestGetHeaderValue(request: &HttpRequest, header_name: String) -> String {
    request
        .headers
        .lock()
        .expect("not poisoned")
        .get(HeaderName::try_from(header_name).expect("valid header name"))
        .expect("header value present")
        .to_str()
        .expect("value is a string")
        .to_string()
}

#[bridge_fn]
fn TESTING_ChatRequestGetBody(request: &HttpRequest) -> Vec<u8> {
    request
        .body
        .clone()
        .map(|b| b.into_vec())
        .unwrap_or_default()
}

#[bridge_fn]
fn TESTING_ChatService_InjectRawServerRequest(chat: &AuthChat, bytes: &[u8]) {
    let request_proto = <chat::RequestProto as prost::Message>::decode(bytes)
        .expect("invalid protobuf cannot use this endpoint to test");
    chat.synthetic_request_tx
        .blocking_send(chat::ws::ServerEvent::fake(request_proto))
        .expect("not closed");
}

#[bridge_fn]
fn TESTING_ChatService_InjectConnectionInterrupted(chat: &AuthChat) {
    chat.synthetic_request_tx
        .blocking_send(chat::ws::ServerEvent::Stopped(ChatServiceError::WebSocket(
            WebSocketServiceError::ChannelClosed,
        )))
        .expect("not closed");
}

#[bridge_fn]
fn TESTING_ChatService_InjectIntentionalDisconnect(chat: &AuthChat) {
    chat.synthetic_request_tx
        .blocking_send(chat::ws::ServerEvent::Stopped(
            ChatServiceError::ServiceIntentionallyDisconnected,
        ))
        .expect("not closed");
}

#[bridge_fn(jni = false, ffi = false)]
fn TESTING_ServerMessageAck_Create() -> ServerMessageAck {
    ServerMessageAck::new(Box::new(|_| Box::pin(std::future::ready(Ok(())))))
}
