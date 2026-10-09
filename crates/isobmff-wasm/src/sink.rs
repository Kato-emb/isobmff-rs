//! [`OpfsSink`], a file of the origin private file system written as a [`Write`]

use std::io::{self, Write};

use web_sys::FileSystemSyncAccessHandle;

use crate::source::{js_failure, js_integer};

/// A file of the origin private file system written from where the cursor of its handle stands, one `FileSystemSyncAccessHandle.write` per `Write::write`
#[derive(Debug)]
pub(crate) struct OpfsSink(pub(crate) FileSystemSyncAccessHandle);

impl Write for OpfsSink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.0.write_with_u8_array(buffer).map_err(js_failure)?;
        usize::try_from(js_integer(written)?).map_err(io::Error::other)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush().map_err(js_failure)
    }
}
