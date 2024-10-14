//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

package org.signal.libsignal.internal;

import static org.junit.Assert.*;

import java.nio.ByteBuffer;
import java.util.concurrent.ExecutionException;
import org.junit.Test;

public class BridgingTest {
  @Test
  public void testErrorOnBorrow() throws Exception {
    assertThrows(
        IllegalArgumentException.class, () -> NativeTesting.TESTING_ErrorOnBorrowSync(null));
    assertThrows(
        IllegalArgumentException.class, () -> NativeTesting.TESTING_ErrorOnBorrowAsync(null));
    assertThrows(
        IllegalArgumentException.class,
        () -> NativeTesting.TESTING_ErrorOnBorrowIo(-1, null).get());
  }

  @Test
  public void testPanicOnBorrow() throws Exception {
    assertThrows(AssertionError.class, () -> NativeTesting.TESTING_PanicOnBorrowSync(null));
    assertThrows(AssertionError.class, () -> NativeTesting.TESTING_PanicOnBorrowAsync(null));
    assertThrows(AssertionError.class, () -> NativeTesting.TESTING_PanicOnBorrowIo(-1, null).get());
  }

  @Test
  public void testPanicOnLoad() throws Exception {
    assertThrows(AssertionError.class, () -> NativeTesting.TESTING_PanicOnLoadSync(null, null));
    assertThrows(AssertionError.class, () -> NativeTesting.TESTING_PanicOnLoadAsync(null, null));
    ExecutionException e =
        assertThrows(
            ExecutionException.class,
            () -> NativeTesting.TESTING_PanicOnLoadIo(-1, null, null).get());
    assertTrue(e.getCause().toString(), e.getCause() instanceof AssertionError);
  }

  @Test
  public void testPanicInBody() throws Exception {
    assertThrows(AssertionError.class, () -> NativeTesting.TESTING_PanicInBodySync(null));
    assertThrows(AssertionError.class, () -> NativeTesting.TESTING_PanicInBodyAsync(null));
    ExecutionException e =
        assertThrows(
            ExecutionException.class, () -> NativeTesting.TESTING_PanicInBodyIo(-1, null).get());
    assertTrue(e.getCause().toString(), e.getCause() instanceof AssertionError);
  }

  @Test
  public void testErrorOnReturn() throws Exception {
    assertThrows(
        IllegalArgumentException.class, () -> NativeTesting.TESTING_ErrorOnReturnSync(null));
    assertThrows(
        IllegalArgumentException.class, () -> NativeTesting.TESTING_ErrorOnReturnAsync(null));
    ExecutionException e =
        assertThrows(
            ExecutionException.class, () -> NativeTesting.TESTING_ErrorOnReturnIo(-1, null).get());
    assertTrue(e.getCause().toString(), e.getCause() instanceof IllegalArgumentException);
  }

  @Test
  public void testPanicOnReturn() throws Exception {
    assertThrows(AssertionError.class, () -> NativeTesting.TESTING_PanicOnReturnSync(null));
    assertThrows(AssertionError.class, () -> NativeTesting.TESTING_PanicOnReturnAsync(null));
    ExecutionException e =
        assertThrows(
            ExecutionException.class, () -> NativeTesting.TESTING_PanicOnReturnIo(-1, null).get());
    assertTrue(e.getCause().toString(), e.getCause() instanceof AssertionError);
  }

  @Test
  public void testReturnStringArray() {
    assertArrayEquals(
        NativeTesting.TESTING_ReturnStringArray(), new String[] {"easy", "as", "ABC", "123"});
  }

  @Test
  public void testProcessBytestringArray() {
    ByteBuffer first = ByteBuffer.allocateDirect(3);
    first.put(new byte[] {1, 2, 3});
    ByteBuffer empty = ByteBuffer.allocateDirect(0);
    ByteBuffer second = ByteBuffer.allocateDirect(3);
    second.put(new byte[] {4, 5, 6});
    byte[][] result =
        NativeTesting.TESTING_ProcessBytestringArray(new ByteBuffer[] {first, empty, second});
    assertArrayEquals(result, new byte[][] {{1, 2, 3, 1, 2, 3}, {}, {4, 5, 6, 4, 5, 6}});
  }

  @Test
  public void testProcessEmptyBytestringArray() {
    assertArrayEquals(
        NativeTesting.TESTING_ProcessBytestringArray(new ByteBuffer[] {}), new byte[][] {});
  }
}
