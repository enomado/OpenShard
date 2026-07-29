# crates/client

Client-side crates live here. Nothing yet: the shard is server-only today, and
the client is the stock UO client speaking the wire protocol from
[`crates/common/protocol`](../common/protocol).

Anything that lands here — a map editor, a headless test client, a launcher —
talks to the server through `crates/common` and never depends on
`crates/server`.
