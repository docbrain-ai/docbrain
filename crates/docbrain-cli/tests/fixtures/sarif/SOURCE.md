# sarif-schema-2.1.0.json

The SARIF 2.1.0 JSON schema the tests validate `docbrain status --base --sarif` against.

- Source: https://github.com/microsoft/sarif-sdk/blob/e95d91299d906efade256b7d4a12dccaaa8058e9/src/Sarif/Schemata/sarif-2.1.0.json
- Commit: `e95d91299d906efade256b7d4a12dccaaa8058e9` (2026-06-23), the latest to touch the file when it was
  vendored on 2026-11-08.
- sha256: `0de6555c956d0e2081bb40f0b877f275634cba28a6f8954e6d858a49138143d6`
- Licence: MIT, the licence of the source repository, reproduced below.
- Draft: JSON Schema 2020-12, as the file's `$schema` says; the tests validate with that draft.

The OASIS errata01 schema
(https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json, draft-04) is
governed by the OASIS IPR policy, which does not plainly grant redistribution here, so it is not vendored. This
file differs from it in these rules only: each GUID's `pattern` is `format: uuid`; `type: string` beside a
string `enum` is dropped; `runs` may not be `null`; and `region` no longer requires one of `startLine`,
`charOffset` or `byteOffset`. The tests assert `startLine` on every region themselves.

## Licence

```
Copyright (c) Microsoft Corporation

All rights reserved.

MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and
to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of
the Software.

THE SOFTWARE IS PROVIDED *AS IS*, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF
CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
IN THE SOFTWARE.
```
