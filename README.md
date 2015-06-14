# formdata

[![Build Status](https://travis-ci.org/mikedilger/formdata.svg?branch=master)](https://travis-ci.org/mikedilger/formdata)

Documentation is available at https://mikedilger.github.io/formdata

This library provides a function for parsing a stream in `multipart/form-data` format,
such as what HTTP user agents (browsers) send to HTTP servers via the POST method
in response to HTTP forms with enctype="multipart/form-data".  These streams typically
contain embedded uploaded files.  This library separates those files and streams them
to disk.

More TBD.
