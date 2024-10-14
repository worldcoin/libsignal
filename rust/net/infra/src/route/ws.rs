//
// Copyright 2024 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use http::uri::PathAndQuery;
use tungstenite::protocol::WebSocketConfig;

use crate::route::{ReplaceFragment, RouteProvider, SimpleRoute};

#[derive(Clone, Debug)]
pub struct WebSocketRouteFragment {
    /// Protocol-level configuration.
    pub ws_config: WebSocketConfig,
    /// The HTTP path to use when establishing the websocket connection.
    pub endpoint: PathAndQuery,
}

pub type WebSocketRoute<H> = SimpleRoute<WebSocketRouteFragment, H>;

#[derive(Debug)]
pub struct WebSocketProvider<P> {
    pub(crate) fragment: WebSocketRouteFragment,
    pub(crate) inner: P,
}

impl<P: RouteProvider> RouteProvider for WebSocketProvider<P> {
    type Route = WebSocketRoute<P::Route>;

    fn routes(&self) -> impl Iterator<Item = Self::Route> + '_ {
        self.inner.routes().map(|route| WebSocketRoute {
            inner: route,
            fragment: self.fragment.clone(),
        })
    }
}

impl<R: ReplaceFragment<S>, S> ReplaceFragment<S> for WebSocketRoute<R> {
    type Replacement<T> = WebSocketRoute<R::Replacement<T>>;

    fn replace<T>(self, make_fragment: impl FnOnce(S) -> T) -> Self::Replacement<T> {
        let Self { inner, fragment } = self;
        WebSocketRoute {
            inner: inner.replace(make_fragment),
            fragment,
        }
    }
}

/// Manual impl because [`tungstenite::protocol::WebSocketConfig`] doesn't
/// implement [`PartialEq`].
impl PartialEq for WebSocketRouteFragment {
    fn eq(&self, other: &Self) -> bool {
        self.endpoint == other.endpoint && ws_config_eq(self.ws_config, other.ws_config)
    }
}

#[allow(deprecated)]
fn ws_config_eq(lhs: WebSocketConfig, rhs: WebSocketConfig) -> bool {
    let WebSocketConfig {
        max_send_queue,
        write_buffer_size,
        max_write_buffer_size,
        max_message_size,
        max_frame_size,
        accept_unmasked_frames,
    } = lhs;

    max_send_queue == rhs.max_send_queue
        && write_buffer_size == rhs.write_buffer_size
        && max_write_buffer_size == rhs.max_write_buffer_size
        && max_message_size == rhs.max_message_size
        && max_frame_size == rhs.max_frame_size
        && accept_unmasked_frames == rhs.accept_unmasked_frames
}
