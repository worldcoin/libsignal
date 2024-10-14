//
// Copyright (C) 2024 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use std::fmt::Debug;

use crate::backup::frame::RecipientId;
use crate::backup::method::LookupPair;
use crate::backup::recipient::DestinationKind;
use crate::backup::time::Timestamp;
use crate::backup::{serialize, TryFromWith};
use crate::proto::backup as proto;

/// Validated version of [`proto::AdHocCall`].
#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct AdHocCall<Recipient> {
    pub id: CallId,
    pub timestamp: Timestamp,
    pub recipient: Recipient,
}

/// Validated version of [`proto::IndividualCall`].
#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct IndividualCall {
    pub id: Option<CallId>,
    pub call_type: CallType,
    pub state: IndividualCallState,
    pub outgoing: bool,
    pub started_at: Timestamp,
    pub read: bool,
}

/// Validated version of [`proto::GroupCall`].
#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct GroupCall<Recipient> {
    pub id: Option<CallId>,
    pub state: GroupCallState,
    pub started_call_recipient: Option<Recipient>,
    pub ringer_recipient: Option<Recipient>,
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
    pub read: bool,
}

/// An identifier for a call.
///
/// This is not referenced as a foreign key from elsewhere in a backup, but
/// corresponds to shared state across conversation members for a given call.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord, serde::Serialize)]
pub struct CallId(u64);

#[derive(Debug, displaydoc::Display, thiserror::Error)]
#[cfg_attr(test, derive(PartialEq))]
pub enum CallError {
    /// call starter {0:?} not found,
    UnknownCallStarter(RecipientId),
    /// call starter {0:?} is a {1:?}, not a contact or self
    InvalidCallStarter(RecipientId, DestinationKind),
    /// no record for ringer {0:?}
    NoRingerRecipient(RecipientId),
    /// ringer {0:?} is a {1:?}, not a contact or self
    InvalidRingerRecipient(RecipientId, DestinationKind),
    /// no record for ad-hoc {0:?}
    NoAdHocRecipient(RecipientId),
    /// ad-hoc recipient {0:?} is not a call link
    InvalidAdHocRecipient(RecipientId),
    /// call type is UNKNOWN_TYPE
    UnknownType,
    /// call state is UNKNOWN_STATE
    UnknownState,
    /// call direction is UNKNOWN_DIRECTION
    UnknownDirection,
}

#[derive(Debug, displaydoc::Display, thiserror::Error)]
#[cfg_attr(test, derive(PartialEq))]
pub enum CallLinkError {
    /// call link restrictions is UNKNOWN
    UnknownRestrictions,
    /// expected {CALL_LINK_ROOT_KEY_LEN:?}-byte root key, found {0} bytes
    InvalidRootKey(usize),
    /// admin key was present but empty
    InvalidAdminKey,
}

#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq))]
pub enum CallType {
    Audio,
    Video,
}

#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq))]
pub enum IndividualCallState {
    Accepted,
    NotAccepted,
    MissedByNotificationProfile,
    Missed,
}

#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq))]
pub enum GroupCallState {
    /// No ring
    Generic,
    /// No ring, joined
    Joined,
    /// Incoming ring currently ongoing
    Ringing,
    /// Incoming ring, accepted
    Accepted,
    /// Incoming ring, missed
    Missed,
    /// Incoming ring, declined
    Declined,
    /// Incoming ring, auto-declined
    MissedByNotificationProfile,
    /// Outgoing ring was started
    OutgoingRing,
}

const CALL_LINK_ROOT_KEY_LEN: usize = 16;
type CallLinkRootKey = [u8; CALL_LINK_ROOT_KEY_LEN];

/// Validated version of [`proto::CallLink`].
#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct CallLink {
    pub admin_approval: bool,
    #[serde(with = "hex")]
    pub root_key: CallLinkRootKey,
    #[serde(serialize_with = "serialize::optional_hex")]
    pub admin_key: Option<Vec<u8>>,
    pub expiration: Timestamp,
    pub name: String,
}

impl TryFrom<proto::IndividualCall> for IndividualCall {
    type Error = CallError;

    fn try_from(call: proto::IndividualCall) -> Result<Self, Self::Error> {
        let proto::IndividualCall {
            callId,
            type_,
            state,
            direction,
            startedCallTimestamp,
            read,
            special_fields: _,
        } = call;

        let outgoing = {
            use proto::individual_call::Direction;
            match direction.enum_value_or_default() {
                Direction::UNKNOWN_DIRECTION => return Err(CallError::UnknownDirection),
                Direction::INCOMING => false,
                Direction::OUTGOING => true,
            }
        };

        let call_type = {
            use proto::individual_call::Type;
            match type_.enum_value_or_default() {
                Type::UNKNOWN_TYPE => return Err(CallError::UnknownType),
                Type::AUDIO_CALL => CallType::Audio,
                Type::VIDEO_CALL => CallType::Video,
            }
        };

        let state = {
            use proto::individual_call::State;
            match state.enum_value_or_default() {
                State::UNKNOWN_STATE => return Err(CallError::UnknownState),
                State::ACCEPTED => IndividualCallState::Accepted,
                State::MISSED => IndividualCallState::Missed,
                State::NOT_ACCEPTED => IndividualCallState::NotAccepted,
                State::MISSED_NOTIFICATION_PROFILE => {
                    IndividualCallState::MissedByNotificationProfile
                }
            }
        };

        let started_at = Timestamp::from_millis(startedCallTimestamp, "Call.timestamp");
        let id = callId.map(CallId);

        Ok(Self {
            id,
            call_type,
            state,
            started_at,
            outgoing,
            read,
        })
    }
}

impl<C: LookupPair<RecipientId, DestinationKind, R>, R: Clone> TryFromWith<proto::GroupCall, C>
    for GroupCall<R>
{
    type Error = CallError;

    fn try_from_with(call: proto::GroupCall, context: &C) -> Result<Self, Self::Error> {
        let proto::GroupCall {
            callId,
            state,
            startedCallTimestamp,
            ringerRecipientId,
            startedCallRecipientId,
            endedCallTimestamp,
            read,
            special_fields: _,
        } = call;

        let started_call_recipient = startedCallRecipientId
            .map(|id| {
                let id = RecipientId(id);
                let (&starter_kind, starter) = context
                    .lookup_pair(&id)
                    .ok_or(CallError::UnknownCallStarter(id))?;
                if !starter_kind.is_individual() {
                    return Err(CallError::InvalidCallStarter(id, starter_kind));
                }
                Ok(starter.clone())
            })
            .transpose()?;

        let ringer_recipient = ringerRecipientId
            .map(|id| {
                let id = RecipientId(id);
                let (&ringer_kind, ringer) = context
                    .lookup_pair(&id)
                    .ok_or(CallError::NoRingerRecipient(id))?;
                if !ringer_kind.is_individual() {
                    return Err(CallError::InvalidRingerRecipient(id, ringer_kind));
                }
                Ok(ringer.clone())
            })
            .transpose()?;

        let state = {
            use proto::group_call::State;
            match state.enum_value_or_default() {
                State::UNKNOWN_STATE => return Err(CallError::UnknownState),
                State::MISSED => GroupCallState::Missed,
                State::GENERIC => GroupCallState::Generic,
                State::JOINED => GroupCallState::Joined,
                State::RINGING => GroupCallState::Ringing,
                State::ACCEPTED => GroupCallState::Accepted,
                State::DECLINED => GroupCallState::Declined,
                State::MISSED_NOTIFICATION_PROFILE => GroupCallState::MissedByNotificationProfile,
                State::OUTGOING_RING => GroupCallState::OutgoingRing,
            }
        };

        let started_at =
            Timestamp::from_millis(startedCallTimestamp, "GroupCall.startedCallTimestamp");
        let ended_at = Timestamp::from_millis(endedCallTimestamp, "GroupCall.endedCallTimestamp");
        let id = callId.map(CallId);

        Ok(Self {
            id,
            state,
            started_call_recipient,
            ringer_recipient,
            started_at,
            ended_at,
            read,
        })
    }
}

impl<C: LookupPair<RecipientId, DestinationKind, R>, R: Clone + Debug>
    TryFromWith<proto::AdHocCall, C> for AdHocCall<R>
{
    type Error = CallError;

    fn try_from_with(item: proto::AdHocCall, context: &C) -> Result<Self, Self::Error> {
        let proto::AdHocCall {
            callId,
            recipientId,
            state,
            callTimestamp,
            special_fields: _,
        } = item;

        let id = CallId(callId);

        let recipient = RecipientId(recipientId);

        let (kind, reference) = context
            .lookup_pair(&recipient)
            .ok_or(CallError::NoAdHocRecipient(recipient))?;
        let recipient = match kind {
            DestinationKind::CallLink => reference.clone(),
            DestinationKind::Contact
            | DestinationKind::DistributionList
            | DestinationKind::Group
            | DestinationKind::ReleaseNotes
            | DestinationKind::Self_ => return Err(CallError::InvalidAdHocRecipient(recipient)),
        };

        {
            use proto::ad_hoc_call::State;
            match state.enum_value_or_default() {
                State::UNKNOWN_STATE => return Err(CallError::UnknownState),
                State::GENERIC => (),
            }
        };

        let timestamp = Timestamp::from_millis(callTimestamp, "AdHocCall.startedCallTimestamp");

        Ok(Self {
            id,
            timestamp,
            recipient,
        })
    }
}

impl TryFrom<proto::CallLink> for CallLink {
    type Error = CallLinkError;

    fn try_from(value: proto::CallLink) -> Result<Self, Self::Error> {
        let proto::CallLink {
            rootKey,
            adminKey,
            name,
            restrictions,
            expirationMs,
            special_fields: _,
        } = value;

        let root_key = rootKey
            .try_into()
            .map_err(|key: Vec<u8>| CallLinkError::InvalidRootKey(key.len()))?;

        let admin_key = {
            if adminKey.as_deref() == Some(&[]) {
                return Err(CallLinkError::InvalidAdminKey);
            }
            adminKey
        };

        let admin_approval = {
            use proto::call_link::Restrictions;
            match restrictions.enum_value_or_default() {
                Restrictions::UNKNOWN => return Err(CallLinkError::UnknownRestrictions),
                Restrictions::NONE => false,
                Restrictions::ADMIN_APPROVAL => true,
            }
        };
        let expiration = Timestamp::from_millis(expirationMs, "CallLink.expirationMs");

        Ok(Self {
            root_key,
            admin_approval,
            admin_key,
            expiration,
            name,
        })
    }
}

#[cfg(test)]
pub(crate) mod test {
    use protobuf::EnumOrUnknown;
    use test_case::test_case;

    use super::*;
    use crate::backup::time::testutil::MillisecondsSinceEpoch;
    use crate::backup::time::Duration;
    use crate::backup::TryIntoWith as _;

    impl proto::IndividualCall {
        const TEST_ID: CallId = CallId(33333);

        pub(crate) fn test_data() -> Self {
            Self {
                callId: Some(Self::TEST_ID.0),
                state: proto::individual_call::State::ACCEPTED.into(),
                type_: proto::individual_call::Type::VIDEO_CALL.into(),
                direction: proto::individual_call::Direction::OUTGOING.into(),
                startedCallTimestamp: MillisecondsSinceEpoch::TEST_VALUE.0,
                read: true,
                ..Default::default()
            }
        }
    }

    impl proto::GroupCall {
        pub(crate) fn test_data() -> Self {
            Self {
                callId: None,
                ringerRecipientId: Some(proto::Recipient::TEST_ID),
                state: proto::group_call::State::ACCEPTED.into(),
                startedCallTimestamp: MillisecondsSinceEpoch::TEST_VALUE.0,
                endedCallTimestamp: MillisecondsSinceEpoch::TEST_VALUE.0 + 1000,
                read: true,
                ..Default::default()
            }
        }
    }

    pub(crate) const TEST_CALL_LINK_RECIPIENT_ID: RecipientId = RecipientId(987654);
    pub(crate) const NONEXISTENT_RECIPIENT: RecipientId = RecipientId(9999999999999999999);

    impl proto::AdHocCall {
        const TEST_ID: u64 = 888888;

        fn test_data() -> Self {
            Self {
                callId: Self::TEST_ID,
                recipientId: TEST_CALL_LINK_RECIPIENT_ID.0,
                state: proto::ad_hoc_call::State::GENERIC.into(),
                callTimestamp: MillisecondsSinceEpoch::TEST_VALUE.0,
                ..Default::default()
            }
        }
    }

    const TEST_CALL_LINK_ROOT_KEY: CallLinkRootKey = [b'R'; 16];
    const TEST_CALL_LINK_ADMIN_KEY: &[u8] = b"A";
    impl proto::CallLink {
        fn test_data() -> Self {
            Self {
                rootKey: TEST_CALL_LINK_ROOT_KEY.to_vec(),
                adminKey: Some(TEST_CALL_LINK_ADMIN_KEY.to_vec()),
                restrictions: proto::call_link::Restrictions::NONE.into(),
                expirationMs: MillisecondsSinceEpoch::TEST_VALUE.0,
                ..Default::default()
            }
        }
    }

    impl CallLink {
        pub(crate) fn from_proto_test_data() -> Self {
            Self {
                admin_approval: false,
                root_key: TEST_CALL_LINK_ROOT_KEY,
                admin_key: Some(TEST_CALL_LINK_ADMIN_KEY.to_vec()),
                expiration: Timestamp::test_value(),
                name: "".to_string(),
            }
        }
    }

    struct TestContext;

    impl LookupPair<RecipientId, DestinationKind, RecipientId> for TestContext {
        fn lookup_pair<'a>(
            &'a self,
            key: &'a RecipientId,
        ) -> Option<(&'a DestinationKind, &'a RecipientId)> {
            match key {
                RecipientId(proto::Recipient::TEST_ID) => Some((&DestinationKind::Contact, key)),
                &TEST_CALL_LINK_RECIPIENT_ID => Some((&DestinationKind::CallLink, key)),
                _ => None,
            }
        }
    }

    #[test]
    fn valid_individual_call() {
        assert_eq!(
            proto::IndividualCall::test_data().try_into(),
            Ok(IndividualCall {
                id: Some(proto::IndividualCall::TEST_ID),
                call_type: CallType::Video,
                state: IndividualCallState::Accepted,
                outgoing: true,
                started_at: Timestamp::test_value(),
                read: true,
            })
        );
    }

    #[test_case(|x| x.type_ = EnumOrUnknown::default() => Err(CallError::UnknownType); "unknown type")]
    #[test_case(|x| x.state = EnumOrUnknown::default() => Err(CallError::UnknownState); "unknown state")]
    #[test_case(|x| x.direction = EnumOrUnknown::default() => Err(CallError::UnknownDirection); "unknown_direction")]
    fn individual_call(modifier: fn(&mut proto::IndividualCall)) -> Result<(), CallError> {
        let mut call = proto::IndividualCall::test_data();
        modifier(&mut call);
        call.try_into().map(|_: IndividualCall| ())
    }

    #[test]
    fn valid_group_call() {
        assert_eq!(
            proto::GroupCall::test_data().try_into_with(&TestContext),
            Ok(GroupCall {
                id: None,
                state: GroupCallState::Accepted,
                started_call_recipient: None,
                ringer_recipient: Some(RecipientId(proto::Recipient::TEST_ID)),
                started_at: Timestamp::test_value(),
                ended_at: Timestamp::test_value() + Duration::from_millis(1000),
                read: true,
            })
        );
    }

    #[test_case(|x| x.ringerRecipientId = None => Ok(()); "no ringer")]
    #[test_case(|x| {
        x.ringerRecipientId = Some(NONEXISTENT_RECIPIENT.0)
    } => Err(CallError::NoRingerRecipient(NONEXISTENT_RECIPIENT)); "nonexistent ringer")]
    #[test_case(|x| {
        x.ringerRecipientId = Some(TEST_CALL_LINK_RECIPIENT_ID.0)
    } => Err(CallError::InvalidRingerRecipient(TEST_CALL_LINK_RECIPIENT_ID, DestinationKind::CallLink)); "invalid ringer")]
    #[test_case(|x| {
        x.startedCallRecipientId = Some(proto::Recipient::TEST_ID)
    } => Ok(()); "has call starter")]
    #[test_case(|x| {
        x.startedCallRecipientId = Some(NONEXISTENT_RECIPIENT.0)
    } => Err(CallError::UnknownCallStarter(NONEXISTENT_RECIPIENT)); "nonexistent call starter")]
    #[test_case(|x| {
        x.startedCallRecipientId = Some(TEST_CALL_LINK_RECIPIENT_ID.0)
    } => Err(CallError::InvalidCallStarter(TEST_CALL_LINK_RECIPIENT_ID, DestinationKind::CallLink)); "invalid call starter")]
    #[test_case(|x| x.state = EnumOrUnknown::default() => Err(CallError::UnknownState); "unknown_state")]
    fn group_call(modifier: fn(&mut proto::GroupCall)) -> Result<(), CallError> {
        let mut call = proto::GroupCall::test_data();
        modifier(&mut call);
        call.try_into_with(&TestContext).map(|_: GroupCall<_>| ())
    }

    #[test]
    fn valid_ad_hoc_call() {
        assert_eq!(
            proto::AdHocCall::test_data().try_into_with(&TestContext),
            Ok(AdHocCall {
                id: CallId(proto::AdHocCall::TEST_ID),
                timestamp: Timestamp::test_value(),
                recipient: TEST_CALL_LINK_RECIPIENT_ID,
            })
        );
    }

    #[test_case(|x| x.state = EnumOrUnknown::default() => Err(CallError::UnknownState); "unknown state")]
    #[test_case(
        |x| x.recipientId = NONEXISTENT_RECIPIENT.0 => Err(CallError::NoAdHocRecipient(NONEXISTENT_RECIPIENT));
        "invalid_ad_hoc_recipient"
    )]
    #[test_case(
        |x| x.recipientId = proto::Recipient::TEST_ID => Err(CallError::InvalidAdHocRecipient(RecipientId(proto::Recipient::TEST_ID)));
        "ad_hoc_recipient_not_call"
    )]
    fn ad_hoc_call(modifier: impl FnOnce(&mut proto::AdHocCall)) -> Result<(), CallError> {
        let mut call = proto::AdHocCall::test_data();
        modifier(&mut call);
        call.try_into_with(&TestContext).map(|_: AdHocCall<_>| ())
    }

    #[test]
    fn valid_call_link() {
        assert_eq!(
            proto::CallLink::test_data().try_into(),
            Ok(CallLink::from_proto_test_data())
        );
    }

    #[test_case(|x| x.rootKey = vec![123] => Err(CallLinkError::InvalidRootKey(1)); "invalid_root_key")]
    #[test_case(|x| x.adminKey = Some(vec![]) => Err(CallLinkError::InvalidAdminKey); "invalid_admin_key")]
    #[test_case(|x| x.adminKey = None => Ok(()); "no_admin_key")]
    #[test_case(|x| x.restrictions = EnumOrUnknown::default() => Err(CallLinkError::UnknownRestrictions); "unknown_restrictions")]
    fn call_link(modifier: fn(&mut proto::CallLink)) -> Result<(), CallLinkError> {
        let mut link = proto::CallLink::test_data();
        modifier(&mut link);
        link.try_into().map(|_: CallLink| ())
    }
}
