// Copyright © 2015 by Michael Dilger (of New Zealand)
// This code is licensed under the MIT license (see LICENSE-MIT for details)

use std::path::PathBuf;
use std::ops::Drop;
use mime::Mime;
use tempdir::TempDir;
use error::Error;
use textnonce::TextNonce;

/// An uploaded file that was received as part of `multipart/form-data` parsing.
///
/// Files are streamed to disk because they may not fit in memory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UploadedFile {
    /// The temporary file where the data was saved.
    pub path: PathBuf,
    /// The filename that was specified in the data, unfiltered. It may or may not be legal on the
    /// local filesystem.
    pub filename: Option<String>,
    /// The unvalidated content-type that was specified in the data.
    pub content_type: Mime,
    /// The size of the file.
    pub size: usize,
    // The temporary directory the upload was put into, saved for the Drop trait
    tempdir: PathBuf,
}

impl UploadedFile {
    pub fn new(content_type: Mime) -> Result<UploadedFile,Error> {
        // Setup a file to capture the contents.
        let tempdir = try!(TempDir::new("formdata")).into_path();
        let mut path = tempdir.clone();
        path.push(TextNonce::sized_urlsafe(32).unwrap().into_string());
        Ok(UploadedFile {
            path: path,
            filename: None,
            content_type: content_type,
            size: 0,
            tempdir: tempdir,
        })
    }
}

impl Drop for UploadedFile {
    fn drop(&mut self) {
        let _ = ::std::fs::remove_file(&self.path);
        let _ = ::std::fs::remove_dir(&self.tempdir);
    }
}
