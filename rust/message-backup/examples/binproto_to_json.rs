//
// Copyright (C) 2024 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use clap::Parser;
use clap_stdin::FileOrStdin;
use futures::io::AllowStdIo;

#[derive(Parser)]
/// Compresses and encrypts an unencrypted backup file.
struct CliArgs {
    /// the file to read from, or '-' to read from stdin
    filename: FileOrStdin,
}

fn main() {
    let CliArgs { filename } = CliArgs::parse();

    eprintln!("reading from {:?}", filename.source);

    let json_array =
        futures::executor::block_on(libsignal_message_backup::backup::convert_to_json(
            AllowStdIo::new(filename.into_reader().expect("failed to open")),
        ))
        .expect("failed to convert");

    print!("{:#}", serde_json::Value::Array(json_array));
}
