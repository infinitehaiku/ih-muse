# Muse contract

What a Muse sends to Poet and the rules Poet applies to it. The Rust types in
`ih-muse-proto` are the reference; this page explains the rules that are not
obvious from the types.

## Dashboard definitions

A Muse describes the default dashboards of what it monitors as
`DashboardDefinition` values (JSON Schema:
`schemas/dashboard-definition.schema.json`; examples: `examples/dashboards/`).
Check a file offline with `ih-muse-cli dashboard check <file>`.

Dashboards have no count limits:

- a definition holds any number of panels and blocks;
- a panel holds any number of `filters`, and a grouped panel (`group_by`)
  shows any number of series (`top_n`, at least 1; 5 when left out);
- a Muse may define any number of dashboards.

Layout: Kabuki draws a definition as a Measurements row (one tile per golden
signal, fed by the panels that carry that signal), then the `blocks` in order.
Each panel's chart is drawn once: in the one block that lists it, else under
Measurements. So a panel in no block needs a golden signal, and a block may
list a golden-signal panel (its chart sits in the block, its value still feeds
the tile; the Kubernetes cluster's "Health now" does this). A definition
without blocks leaves the layout to the renderer. A panel listed in two blocks
is refused.

Only input sizes are bounded, so that one faulty Muse cannot flood Poet: the
byte length of ids, titles, descriptions, block texts and metric names (see the
schema), the entries of each recognition list (32), and the request size of
the transport.

### Delivery

Definitions ride the `dashboards` field of a `GraphBatch`. One batch carries at
most 32 definitions. This is a delivery chunk, not a limit: a Muse with more
sends them over several batches. The `DashboardDelivery` helper (Rust
`ih_muse::dashboards::DashboardDelivery`, Python `ih_muse.DashboardDelivery`)
does this for you:

1. `attach(batch)` puts the first chunk no Poet has acknowledged yet on the
   batch. Every batch carries it until a Poet acknowledges one of them, so a
   queue that drops its oldest batches cannot lose it.
2. `acknowledge(batch)` after a Poet accepted the batch marks the chunk it
   carried as delivered; the next batch carries the next chunk.
3. Once every chunk is delivered, batches leave the field out.
4. `resend()` starts again from the first chunk, for example after the Muse
   changed its definitions.

Poet keeps every revision of every definition id. The same `(id, revision)`
sent again changes nothing; to change a dashboard, raise its `revision`.

## Naming the exact element of a log line or span

Poet attaches each OpenTelemetry log record and span to the most specific
element a Muse reports, using the OpenTelemetry semantic conventions on the
record or its resource (`host.name`, `process.pid`, `k8s.pod.uid`,
`container.id`, ...).

When the conventions cannot name the element a record is about (a disk volume,
a network interface, a battery), a Muse may name it itself with the attribute
`ih.element.key`, on the record or on its resource. The value is the Muse's own
key of that element, exactly as the Muse reports the element in its graph
batches (for example `disk:/Volumes/Data` or `network:en0`).

The rule Poet applies:

- Poet first resolves the conventions as usual (the host, process or pod of
  the record). The key alone resolves nothing: a record must also carry the
  conventions of the host, process or Kubernetes object it comes from.
- Poet then looks the key up only among the elements that **the same Muse**
  reports, **in the same organization**, and in the same scope as the element
  the conventions found (the host's or process's environment, the Kubernetes
  object's cluster). It never matches another Muse's elements, so two Muses
  using the same key never mix.
- If the Muse has not reported that element yet (a volume mounted a second
  ago), the record stays on the element the conventions found and moves to the
  exact element once the Muse reports it.
- If the key names nothing the Muse reports, the record stays on the element
  the conventions found.

Example log record attributes from a macOS Muse:

```json
{
  "host.name": "studio.local",
  "ih.element.key": "disk:/Volumes/Data"
}
```
