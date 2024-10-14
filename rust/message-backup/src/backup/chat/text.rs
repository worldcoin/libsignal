//
// Copyright (C) 2024 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use libsignal_core::Aci;

use crate::backup::serialize::{self, UnorderedList};
use crate::backup::uuid_bytes_to_aci;
use crate::proto::backup as proto;

/// Validated version of [`proto::Text`].
#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct MessageText {
    pub text: String,
    pub ranges: UnorderedList<TextRange>,
}

#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq, Clone))]
pub struct TextRange {
    pub start: Option<u32>,
    pub length: Option<u32>,
    pub effect: TextEffect,
}

#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq, Clone))]
pub enum TextEffect {
    MentionAci(#[serde(serialize_with = "serialize::service_id_as_string")] Aci),
    Style(#[serde(serialize_with = "serialize::enum_as_string")] proto::body_range::Style),
}

#[derive(Debug, displaydoc::Display, thiserror::Error)]
#[cfg_attr(test, derive(PartialEq))]
pub enum TextError {
    /// mention had invalid ACI
    MentionInvalidAci,
    /// BodyRange.associatedValue is a oneof but has no value
    NoAssociatedValueForBodyRange,
}

impl TryFrom<proto::Text> for MessageText {
    type Error = TextError;

    fn try_from(value: proto::Text) -> Result<Self, Self::Error> {
        let proto::Text {
            body,
            bodyRanges,
            special_fields: _,
        } = value;

        let ranges = bodyRanges
            .into_iter()
            .map(|range| {
                let proto::BodyRange {
                    start,
                    length,
                    associatedValue,
                    special_fields: _,
                } = range;
                use proto::body_range::AssociatedValue;
                let effect =
                    match associatedValue.ok_or(TextError::NoAssociatedValueForBodyRange)? {
                        AssociatedValue::MentionAci(aci) => TextEffect::MentionAci(
                            uuid_bytes_to_aci(aci).map_err(|_| TextError::MentionInvalidAci)?,
                        ),
                        AssociatedValue::Style(style) => {
                            // All style values are valid
                            TextEffect::Style(style.enum_value_or_default())
                        }
                    };
                Ok(TextRange {
                    start,
                    length,
                    effect,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { text: body, ranges })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::backup::testutil::TEST_MESSAGE_TEXT;

    impl proto::Text {
        pub(crate) fn test_data() -> Self {
            Self {
                body: TEST_MESSAGE_TEXT.to_string(),
                bodyRanges: vec![proto::BodyRange {
                    start: Some(2),
                    length: Some(5),
                    associatedValue: Some(proto::body_range::AssociatedValue::Style(
                        proto::body_range::Style::MONOSPACE.into(),
                    )),
                    special_fields: Default::default(),
                }],
                special_fields: Default::default(),
            }
        }
    }

    impl MessageText {
        pub(crate) fn from_proto_test_data() -> Self {
            Self {
                text: TEST_MESSAGE_TEXT.to_string(),
                ranges: vec![TextRange {
                    start: Some(2),
                    length: Some(5),
                    effect: TextEffect::Style(proto::body_range::Style::MONOSPACE),
                }]
                .into(),
            }
        }
    }

    #[test]
    fn valid_text() {
        assert_eq!(
            proto::Text::test_data().try_into(),
            Ok(MessageText::from_proto_test_data())
        );
    }

    #[test]
    fn ranges_are_sorted_when_serialized() {
        let range1 = TextRange {
            start: Some(2),
            length: Some(5),
            effect: TextEffect::Style(proto::body_range::Style::MONOSPACE),
        };
        let range2 = TextRange {
            start: Some(10),
            length: Some(2),
            effect: TextEffect::Style(proto::body_range::Style::BOLD),
        };

        let message1 = MessageText {
            ranges: vec![range1.clone(), range2.clone()].into(),
            ..MessageText::from_proto_test_data()
        };
        let message2 = MessageText {
            ranges: vec![range2, range1].into(),
            ..MessageText::from_proto_test_data()
        };

        assert_eq!(
            serde_json::to_string_pretty(&message1).expect("valid"),
            serde_json::to_string_pretty(&message2).expect("valid"),
        );
    }
}
