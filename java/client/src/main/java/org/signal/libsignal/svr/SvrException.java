//
// Copyright 2024 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

package org.signal.libsignal.svr;

public class SvrException extends Exception {
  public SvrException(String message) {
    super(message);
  }
}
