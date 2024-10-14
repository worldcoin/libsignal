This crate contains a validator for Signal's backup file format. Structurally, these files are an
encrypted series of length-delimited protos, following the rules described in [backup.proto](src/proto/backup.proto).

The crate is used in some specific ways that have led to unusual design decisions, particularly concerning the model types in the `backup` submodule:

- We use the `protobuf-rs` crate instead of the more popular, better-supported, and used-elsewhere-in-libsignal `prost` because `prost` doesn't support doing anything with unknown fields. (We just want to report them, we don't need to store them in every message. But even that doesn't seem interesting to the mainline prost folks. See <https://github.com/tokio-rs/prost/issues/2>.)

- The model types roughly parallel the protobuf message types, but they use stronger types for their fields, representing the stronger constraints of a valid backup. For example, a contact's ACI will be represented using `libsignal_core::Aci` rather than a plain protobuf `bytes` (`Vec<u8>` in Rust).

- Since we want to be sure we don't ignore validity constraints, we don't use `..` when matching the protobuf struct types; we always want to explicitly enumerate all the fields. Similarly, since we want the model types to always represent *validated* data, even elsewhere in the crate, we often include a non-public `_limit_construction_to_module` member, forcing construction to go through the conversion from a protobuf message or reuse deliberately-constructed test data (even if they modify the value after).

- The client apps do a lightweight "validation" whenever they generate backups, but they also want to do a full deserialization in their tests (mostly using the "comparable" form described below). Therefore, many of the types nearer the root of the model are parameterized with a `Method`; the two methods are `Store` and `ValidateOnly`, where the latter minimizes memory use by throwing out most of what it reads once it's been validated.

- The model types implement `serde::Serialize`, but are deliberately *not* trying to be a fully value-preserving, round-trip mechanism. Rather, the Serialized form represents a "comparable" form, one that has gone through some canonicalization, so that disparate backup implementations can still be compared. A restore implementation should aim to satisfy `serialize(backup) == serialize(export(restoreFrom(backup)))`.

    This is exposed to the client apps using pretty-printed JSON, since getting good output from a structural diff algorithm is hard and the goal should be "no differences" anyway.

    (The fully value-preserving, round-trip mechanism for serializing a backup is to keep it in the pre-validated protobuf form.)