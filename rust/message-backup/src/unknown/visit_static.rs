//
// Copyright 2024 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use std::collections::HashMap;
use std::ops::ControlFlow;

pub(crate) use libsignal_message_backup_macros::VisitUnknownFields;
use protobuf::{EnumOrUnknown, MessageField, SpecialFields, UnknownFields};

use crate::unknown::{MapKey, Part, Path, UnknownFieldVisitor, UnknownValue};

pub(crate) trait Visitor {
    fn unknown_fields(&mut self, path: Path<'_>, unknown: &UnknownFields) -> ControlFlow<()>;
    fn unknown_enum(&mut self, path: Path<'_>, value: i32) -> ControlFlow<()>;
}

impl<U: UnknownFieldVisitor> Visitor for U {
    fn unknown_fields(&mut self, path: Path<'_>, unknown: &UnknownFields) -> ControlFlow<()> {
        for (tag, _value) in unknown {
            self(path.owned_parts(), UnknownValue::Field { tag })?
        }
        ControlFlow::Continue(())
    }

    fn unknown_enum(&mut self, path: Path<'_>, value: i32) -> ControlFlow<()> {
        self(
            path.owned_parts(),
            UnknownValue::EnumValue { number: value },
        )
    }
}

pub(crate) trait VisitUnknownFields {
    /// Calls the visitor for each unknown field in the message.
    fn visit_unknown_fields(&self, path: Path<'_>, visitor: &mut impl Visitor) -> ControlFlow<()>;
}

impl<V: VisitUnknownFields> VisitUnknownFields for &V {
    fn visit_unknown_fields(&self, path: Path<'_>, visitor: &mut impl Visitor) -> ControlFlow<()> {
        V::visit_unknown_fields(self, path, visitor)
    }
}

impl<E: protobuf::Enum> VisitUnknownFields for EnumOrUnknown<E> {
    fn visit_unknown_fields(&self, path: Path<'_>, visitor: &mut impl Visitor) -> ControlFlow<()> {
        match self.enum_value() {
            Ok(_) => ControlFlow::Continue(()),
            Err(v) => visitor.unknown_enum(path, v),
        }
    }
}

impl<E: VisitUnknownFields> VisitUnknownFields for Box<E> {
    fn visit_unknown_fields(&self, path: Path<'_>, visitor: &mut impl Visitor) -> ControlFlow<()> {
        E::visit_unknown_fields(self, path, visitor)
    }
}

impl<E: VisitUnknownFields> VisitUnknownFields for MessageField<E> {
    fn visit_unknown_fields(&self, path: Path<'_>, visitor: &mut impl Visitor) -> ControlFlow<()> {
        self.0.as_ref().map_or(ControlFlow::Continue(()), |inner| {
            inner.visit_unknown_fields(path, visitor)
        })
    }
}

pub(crate) trait VisitContainerUnknownFields {
    fn visit_unknown_fields_within(
        &self,
        parent_path: Path<'_>,
        field_name: &str,
        visitor: &mut impl Visitor,
    ) -> ControlFlow<()>;
}

impl VisitContainerUnknownFields for SpecialFields {
    fn visit_unknown_fields_within(
        &self,
        parent_path: Path<'_>,
        _field_name: &str,
        visitor: &mut impl Visitor,
    ) -> ControlFlow<()> {
        debug_assert_eq!(_field_name, "special_fields");
        visitor.unknown_fields(parent_path, self.unknown_fields())
    }
}

impl<U: VisitUnknownFields> VisitContainerUnknownFields for Vec<U> {
    fn visit_unknown_fields_within(
        &self,
        parent_path: Path<'_>,
        field_name: &str,
        visitor: &mut impl Visitor,
    ) -> ControlFlow<()> {
        for (index, item) in self.iter().enumerate() {
            let path = Path::Branch {
                parent: &parent_path,
                field_name,
                part: Part::Repeated { index },
            };
            item.visit_unknown_fields(path, visitor)?
        }
        ControlFlow::Continue(())
    }
}

impl<K, V: VisitUnknownFields> VisitContainerUnknownFields for HashMap<K, V>
where
    for<'a> &'a K: Into<MapKey<'a>>,
{
    fn visit_unknown_fields_within(
        &self,
        parent_path: Path<'_>,
        field_name: &str,
        visitor: &mut impl Visitor,
    ) -> ControlFlow<()> {
        for (key, value) in self.iter() {
            let path = Path::Branch {
                parent: &parent_path,
                field_name,
                part: Part::MapValue { key: key.into() },
            };
            value.visit_unknown_fields(path, visitor)?
        }
        ControlFlow::Continue(())
    }
}

impl<U: VisitUnknownFields> VisitContainerUnknownFields for Option<U> {
    fn visit_unknown_fields_within(
        &self,
        parent_path: Path<'_>,
        field_name: &str,
        visitor: &mut impl Visitor,
    ) -> ControlFlow<()> {
        self.as_ref().map_or(ControlFlow::Continue(()), |inner| {
            inner.visit_unknown_fields(
                Path::Branch {
                    parent: &parent_path,
                    field_name,
                    part: Part::Field,
                },
                visitor,
            )
        })
    }
}

macro_rules! no_unknown_fields {
    ($type:path) => {
        impl VisitUnknownFields for $type {
            fn visit_unknown_fields(
                &self,
                _path: Path<'_>,
                _visitor: &mut impl Visitor,
            ) -> ControlFlow<()> {
                ControlFlow::Continue(())
            }
        }
    };
}

no_unknown_fields!(u8);
no_unknown_fields!(u32);
no_unknown_fields!(u64);
no_unknown_fields!(i32);
no_unknown_fields!(i64);
no_unknown_fields!(f32);
no_unknown_fields!(f64);
no_unknown_fields!(bool);
no_unknown_fields!(String);
no_unknown_fields!(Vec<u8>);
