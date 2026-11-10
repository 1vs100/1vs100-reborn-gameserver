# OpenTriviaDB answers (shape recorded, content invented)

These bodies have the shape the API answered on 2026-10-23 (compact JSON, `response_code` and
`results`, every result field Base64-encoded with `encode=base64`; a call for more questions than
are left answers `{"response_code":4,"results":[]}`). The questions in them are invented: the
real ones are CC BY-SA 4.0 data and stay out of the repository.

`13-amount-<n>.json` answers `api.php?amount=<n>&category=13...`; `13-amount-<n>-first.json` and
`-second.json` are its first and second answers.

`stream-amount-<n>[-first|-second].json` answer `api.php?amount=<n>&type=multiple...` without a
category: one pull of every category, each result naming its own (one of them a name the
category list does not have).
