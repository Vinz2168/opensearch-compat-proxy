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
- Streams every other request and response body through untouched, without
  buffering it in memory — a multi-gigabyte `_bulk` request is forwarded
  chunk by chunk as it arrives, not read into RAM first.

## Usage

### Prebuilt binary

Download the binary for your platform from the
[releases page](https://github.com/Vinz2168/opensearch-compat-proxy/releases),
then:

```sh
chmod +x opensearch-compat-proxy
UPSTREAM=http://your-opensearch-host:9200 ./opensearch-compat-proxy
```

### From source

```sh
cargo build --release
UPSTREAM=http://your-opensearch-host:9200 ./target/release/opensearch-compat-proxy
# or, for a quick one-off run:
UPSTREAM=http://your-opensearch-host:9200 cargo run --release
```

### Configuration

All configuration is via environment variables (all optional):

| Variable          | Default                   | Meaning                                   |
|--------------------|----------------------------|--------------------------------------------|
| `UPSTREAM`         | `http://127.0.0.1:9200`    | The real OpenSearch cluster to proxy to    |
| `LISTEN`           | `0.0.0.0:9201`             | Address the proxy listens on               |
| `FAKE_ES_VERSION`  | `7.10.2`                   | Version string reported in `GET /`         |

Point your Elastic client at the proxy's `LISTEN` address instead of
OpenSearch directly — e.g. if you were configuring the client with
`http://opensearch:9200`, point it at `http://<proxy-host>:9201` instead.

### Quick check

```sh
curl -s http://127.0.0.1:9201/ | python3 -m json.tool   # version.number should read 7.10.2
curl -sD - http://127.0.0.1:9201/ -o /dev/null | grep -i x-elastic-product
```

## Streaming

Both directions are streamed without buffering, with one deliberate
exception: the root `GET /` response is small and always buffered, because
its JSON body has to be parsed and rewritten (`version.number`) before being
sent back. Everything else — including large `_bulk` request bodies and
large response bodies — is forwarded chunk by chunk via
`Body::into_data_stream()` / `reqwest::Body::wrap_stream()`, never fully
read into memory. Verified by capturing the raw bytes the proxy sends
upstream for a POST body: correct `Transfer-Encoding: chunked` framing, no
stray `Content-Length` left over from the original request.

Bodyless requests (a plain `GET`/`HEAD`) are detected via the request body's
size hint and sent upstream with no body at all, rather than as an empty
chunked stream.

## What this does *not* fix

Only the header check and the root version string are addressed. API
surface that has diverged between the two projects since the 7.10.2 fork
point (new endpoints/parameters added by either side since 2021) is not
touched by this proxy and will still behave however OpenSearch actually
implements it.

## License

Apache License 2.0.
