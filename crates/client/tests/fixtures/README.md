# CA distrust regression fixtures

`distrusted_ca_keys.pem` contains **public keys only**, from rustls-webpki's
CA distrust patch:

https://github.com/rustls/webpki/tree/8ecdcc76f56b68fda67a6adf7976f5eed5006a4c/src/data/distrusted

The upstream `.spki.der` files contain the contents of a DER SEQUENCE. These
fixtures restore the outer SEQUENCE tag and length and encode the complete
SubjectPublicKeyInfo as PEM. The regression test checks all eight SHA-256 hashes
against Go's corresponding fix for CVE-2026-111943:

https://github.com/golang/go/commit/7f86f957633ee31a1c17a1eb8452ca7a3d9bc141

The test issues new leaf certificates for these public keys under an ephemeral
test CA. This verifies key-based distrust independently of the original CA
names, certificate validity dates, and public root store. No compromised
private keys or live network services are used.

The upstream files are distributed under the ISC license:
https://github.com/rustls/webpki/blob/8ecdcc76f56b68fda67a6adf7976f5eed5006a4c/LICENSE

Copyright 2015 Brian Smith.

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHORS DISCLAIM ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
