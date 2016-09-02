# formdata

[![Build Status](https://travis-ci.org/mikedilger/formdata.svg?branch=master)](https://travis-ci.org/mikedilger/formdata)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

Documentation is available at https://mikedilger.github.io/formdata

This library provides a type for storing `multipart/form-data` data, as well as functions
to stream (read or write) such data over HTTP.

`multipart/form-data` format as described by [RFC 7578](https://tools.ietf.org/html/rfc7578).
HTML forms with enctype=`multipart/form-data` `POST` their data in this format.
This `enctype` is typically used whenever a form has file upload input fields,
as the default `application/x-www-form-urlencoded` cannot handle file uploads.

Whether reading from a stream or writing out to a stream, files are never stored entirely
in memory, but instead streamed through a buffer.
