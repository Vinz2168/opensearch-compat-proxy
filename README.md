# opensearch-compat-proxy

A small transparent reverse proxy that makes an OpenSearch cluster pass the
compatibility checks recent official Elastic clients (Elasticsearch
Python/Java/Node/etc., and anything built on them, like Logstash or NiFi's
Elasticsearch processors) perform before they'll talk to a server at all.

## Why

Since Elasticsearch/Kibana 7.11 (January 2021), Elastic's official clients
refuse to talk to a server unless every response carries an
`X-Elastic-Product: Elasticsearch` header. OpenSearch, being an independent
fork of Elasticsearch 7.10.2, never sends that header, so modern Elastic
clients reject it outright — regardless of how close the underlying REST API
still is.

OpenSearch itself used to ship a `compatibility.override_main_response_version`
cluster setting that made it report `7.10.2` in the root `GET /` response
(which some older, version-checking clients relied on). That setting was
deprecated in OpenSearch 1.x and **removed in 2.0** — there is no
server-side config left to fix this on current OpenSearch. AWS's own
official answer to the header problem is "use OpenSearch's own forked
clients," which isn't always practical if you're stuck with a tool that only
speaks the Elastic client libraries.

This proxy sits in front of OpenSearch and fixes both problems at the wire
level instead:

- Adds `X-Elastic-Product: Elasticsearch` to every response.
- Rewrites `version.number` in the root `GET /` response to a configurable
  fake version (`7.10.2` by default — the last version OpenSearch stayed
  wire-compatible with).
- Passes every other request/response through untouched, streamed.

## Usage

```sh
cargo run --release
```

Environment variables (all optional):

| Variable          | Default                   | Meaning                                   |
|--------------------|----------------------------|--------------------------------------------|
| `UPSTREAM`         | `http://127.0.0.1:9200`    | The real OpenSearch cluster to proxy to    |
| `LISTEN`           | `0.0.0.0:9201`             | Address the proxy listens on               |
| `FAKE_ES_VERSION`  | `7.10.2`                   | Version string reported in `GET /`         |

Point your Elastic client at the proxy's `LISTEN` address instead of
OpenSearch directly.

## Known limitation: request bodies are buffered, not streamed

Response bodies are streamed through untouched (except the one small root
`GET /` body, which has to be buffered to rewrite its JSON). **Request**
bodies, however, are currently read into memory in full before being
forwarded upstream — this keeps the implementation simple and correct, but
means a multi-gigabyte `_bulk` request will be fully buffered in the proxy's
memory rather than streamed through.

If that matters for your workload, swap the `to_bytes(req.into_body(), ...)`
call in `src/main.rs` for a streamed `reqwest::Body` built from
`req.into_body().into_data_stream()`.

## What this does *not* fix

Only the header check and the root version string are addressed. API
surface that has diverged between the two projects since the 7.10.2 fork
point (new endpoints/parameters added by either side since 2021) is not
touched by this proxy and will still behave however OpenSearch actually
implements it.

## License

Apache License 2.0.
