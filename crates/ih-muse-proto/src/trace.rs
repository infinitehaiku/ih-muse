// crates/ih-muse-proto/src/trace.rs

//! W3C Trace Context and sampling, shared by every Infinite Haiku
//! component (browser, Kabuki, Shibuya, Poet and the Muses).
//!
//! - [`TraceContext`] parses and formats a version-00 `traceparent`
//!   (`00-<32 hex trace id>-<16 hex span id>-<2 hex flags>`, lower case,
//!   ids never all zero), makes new traces and child spans.
//! - [`Sampler`] decides which work is kept: the root of a trace samples by
//!   its trace id ([`Sampler::head`], the same answer on every hop that
//!   asks), and every hop keeps its own failed and slow work even when the
//!   trace was not sampled ([`Sampler::keep`]).
//! - [`DeliveryTrace`] is what a Muse sends with a graph delivery (as
//!   headers): its trace context and when the batch was built, so the Poet
//!   that accepts it records the Muse's producer span on its behalf (a
//!   Muse sends no spans of its own).

/// The W3C trace context header.
pub const TRACEPARENT: &str = "traceparent";
/// Head-sampling ratio setting (`0` to `1`; default [`DEFAULT_SAMPLE_RATIO`]).
pub const SAMPLE_RATIO_VARIABLE: &str = "IH_TRACE_SAMPLE_RATIO";
/// Slow-work threshold setting, in milliseconds (default [`DEFAULT_SLOW_MS`]).
pub const SLOW_MS_VARIABLE: &str = "IH_TRACE_SLOW_MS";
/// Share of traces kept when nothing is configured.
pub const DEFAULT_SAMPLE_RATIO: f64 = 0.1;
/// Work slower than this is kept even when its trace was not sampled.
pub const DEFAULT_SLOW_MS: u64 = 2_000;
/// Span attribute saying why an unsampled trace's span was kept
/// (`error` or `slow`); absent on sampled spans.
pub const KEPT_FOR_ATTRIBUTE: &str = "ih.trace.kept_for";

/// One span's place in a trace: the trace id, this span's id and whether
/// the trace is sampled (flag bit 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TraceContext {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub sampled: bool,
}

fn lower_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let mut out = [0u8; N];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Lower-case hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// Parses `N` bytes of lower-case hex (a trace id is 16, a span id 8);
/// `None` for anything else.
pub fn parse_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    lower_hex::<N>(text)
}

/// A new random trace id (never all zero).
pub fn new_trace_id() -> [u8; 16] {
    loop {
        let id = *uuid::Uuid::new_v4().as_bytes();
        if id != [0; 16] {
            return id;
        }
    }
}

/// A new random span id (never all zero).
pub fn new_span_id() -> [u8; 8] {
    loop {
        let mut id = [0u8; 8];
        id.copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[8..]);
        if id != [0; 8] {
            return id;
        }
    }
}

impl TraceContext {
    /// Parses a version-00 `traceparent`; `None` for another version,
    /// upper-case hex, all-zero ids or a malformed value.
    pub fn parse(value: &str) -> Option<Self> {
        let mut parts = value.trim().split('-');
        let (version, trace, span, flags) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || version != "00" {
            return None;
        }
        let flags = lower_hex::<1>(flags)?;
        let trace_id = lower_hex::<16>(trace)?;
        let span_id = lower_hex::<8>(span)?;
        (trace_id != [0; 16] && span_id != [0; 8]).then_some(Self {
            trace_id,
            span_id,
            sampled: flags[0] & 1 == 1,
        })
    }

    /// A new trace (the root span), sampled as `sampler` decides by its id.
    pub fn root(sampler: &Sampler) -> Self {
        let trace_id = new_trace_id();
        Self {
            trace_id,
            span_id: new_span_id(),
            sampled: sampler.head(&trace_id),
        }
    }

    /// A child span of this one: the same trace and decision, a new span id.
    pub fn child(&self) -> Self {
        Self {
            span_id: new_span_id(),
            ..*self
        }
    }

    /// A child of `parent` when there is one, else a new root.
    pub fn continue_or_root(parent: Option<&TraceContext>, sampler: &Sampler) -> Self {
        parent.map_or_else(|| Self::root(sampler), TraceContext::child)
    }

    /// The `traceparent` value naming this span as the parent.
    pub fn traceparent(&self) -> String {
        format!(
            "00-{}-{}-{}",
            hex(&self.trace_id),
            hex(&self.span_id),
            if self.sampled { "01" } else { "00" }
        )
    }

    /// Lower-case hex trace id (32 digits).
    pub fn trace_id_hex(&self) -> String {
        hex(&self.trace_id)
    }

    /// Lower-case hex span id (16 digits).
    pub fn span_id_hex(&self) -> String {
        hex(&self.span_id)
    }
}

/// Why a span is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// Its trace was sampled.
    Sampled,
    /// Not sampled, but the work failed.
    Error,
    /// Not sampled, but the work was slower than the threshold.
    Slow,
}

impl Keep {
    /// The value of [`KEPT_FOR_ATTRIBUTE`]; `None` for sampled spans.
    pub fn kept_for(self) -> Option<&'static str> {
        match self {
            Keep::Sampled => None,
            Keep::Error => Some("error"),
            Keep::Slow => Some("slow"),
        }
    }
}

/// The sampling rules of one process: the head-sampling ratio for traces
/// it starts, and the threshold above which its own unsampled work is
/// kept as slow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampler {
    /// Share of new traces sampled, `0.0..=1.0`.
    pub ratio: f64,
    /// Work at least this long (ns) is kept even when not sampled.
    pub slow_nanos: u64,
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            ratio: DEFAULT_SAMPLE_RATIO,
            slow_nanos: DEFAULT_SLOW_MS * 1_000_000,
        }
    }
}

impl Sampler {
    /// The rules from the settings' text values (`None` or invalid values
    /// keep the defaults; the ratio is clamped to `0..=1`).
    pub fn from_values(ratio: Option<&str>, slow_ms: Option<&str>) -> Self {
        let mut sampler = Self::default();
        if let Some(ratio) = ratio.and_then(|text| text.trim().parse::<f64>().ok()) {
            if ratio.is_finite() {
                sampler.ratio = ratio.clamp(0.0, 1.0);
            }
        }
        if let Some(slow) = slow_ms.and_then(|text| text.trim().parse::<u64>().ok()) {
            sampler.slow_nanos = slow.saturating_mul(1_000_000);
        }
        sampler
    }

    /// The rules from [`SAMPLE_RATIO_VARIABLE`] and [`SLOW_MS_VARIABLE`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_env() -> Self {
        Self::from_values(
            std::env::var(SAMPLE_RATIO_VARIABLE).ok().as_deref(),
            std::env::var(SLOW_MS_VARIABLE).ok().as_deref(),
        )
    }

    /// Whether a trace with `trace_id` is sampled: its last 7 bytes (the
    /// W3C random part) read as a number below `ratio` of their range. Every
    /// process with the same ratio decides the same way.
    pub fn head(&self, trace_id: &[u8; 16]) -> bool {
        if self.ratio >= 1.0 {
            return true;
        }
        if self.ratio <= 0.0 {
            return false;
        }
        let mut random = [0u8; 8];
        random[1..].copy_from_slice(&trace_id[9..]);
        let value = u64::from_be_bytes(random);
        (value as f64) < self.ratio * (1u64 << 56) as f64
    }

    /// Whether to keep one span: sampled traces always; unsampled work
    /// when it failed or took at least the slow threshold.
    pub fn keep(&self, sampled: bool, failed: bool, elapsed_nanos: u64) -> Option<Keep> {
        if sampled {
            Some(Keep::Sampled)
        } else if failed {
            Some(Keep::Error)
        } else if elapsed_nanos >= self.slow_nanos {
            Some(Keep::Slow)
        } else {
            None
        }
    }
}

/// The `tracestate` header.
pub const TRACESTATE: &str = "tracestate";
/// Key of IH's entry in `tracestate` (`ih=c.<ns>`: when a delivery's
/// batch was built).
const TRACESTATE_KEY: &str = "ih";

/// The trace of one Muse delivery, sent with it as the HTTP headers
/// `traceparent` (the Muse's producer span) and `tracestate`
/// (`ih=c.<ns>`: when the batch was built, the start of that span). The
/// delivery body is unchanged, so a Poet that predates tracing accepts it,
/// and the Poet that accepts it records the Muse's span on its behalf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryTrace {
    /// The Muse's producer span.
    pub context: TraceContext,
    /// When the batch was built (ns since the epoch).
    pub created_unix_nano: u64,
}

impl DeliveryTrace {
    /// A new delivery trace, sampled as `sampler` decides.
    pub fn start(sampler: &Sampler, created_unix_nano: u64) -> Self {
        Self {
            context: TraceContext::root(sampler),
            created_unix_nano,
        }
    }

    /// The `tracestate` value carrying the creation time.
    pub fn tracestate(&self) -> String {
        format!("{TRACESTATE_KEY}=c.{}", self.created_unix_nano)
    }

    /// The delivery trace of a request's headers; `None` without a valid
    /// `traceparent`. A missing or foreign `tracestate` leaves the creation
    /// time 0 (unknown).
    pub fn from_headers(traceparent: Option<&str>, tracestate: Option<&str>) -> Option<Self> {
        let context = TraceContext::parse(traceparent?)?;
        let created_unix_nano = tracestate
            .into_iter()
            .flat_map(|value| value.split(','))
            .filter_map(|member| member.trim().split_once('='))
            .find(|(key, _)| *key == TRACESTATE_KEY)
            .and_then(|(_, value)| value.strip_prefix("c."))
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        Some(Self {
            context,
            created_unix_nano,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUE: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn traceparent_round_trips_and_bad_values_are_refused() {
        let context = TraceContext::parse(VALUE).unwrap();
        assert!(context.sampled);
        assert_eq!(context.trace_id_hex(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(context.span_id_hex(), "00f067aa0ba902b7");
        assert_eq!(context.traceparent(), VALUE);
        let unsampled = TraceContext::parse(&VALUE.replace("-01", "-00")).unwrap();
        assert!(!unsampled.sampled);
        for refused in [
            "",
            "garbage",
            "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
            "00-4bf92f3577b34da6a3ce929d0e0e473-00f067aa0ba902b7-01",
        ] {
            assert_eq!(TraceContext::parse(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_child_keeps_the_trace_and_the_decision() {
        let parent = TraceContext::parse(VALUE).unwrap();
        let child = parent.child();
        assert_eq!(child.trace_id, parent.trace_id);
        assert_eq!(child.sampled, parent.sampled);
        assert_ne!(child.span_id, parent.span_id);
        assert_eq!(TraceContext::parse(&child.traceparent()), Some(child));
        let all = Sampler {
            ratio: 1.0,
            ..Sampler::default()
        };
        let root = TraceContext::continue_or_root(None, &all);
        assert!(root.sampled);
        assert_eq!(
            TraceContext::continue_or_root(Some(&parent), &all).trace_id,
            parent.trace_id
        );
    }

    #[test]
    fn head_sampling_follows_the_ratio_and_agrees_across_processes() {
        let tenth = Sampler::from_values(Some("0.1"), None);
        let ids: Vec<[u8; 16]> = (0..20_000).map(|_| new_trace_id()).collect();
        let kept = ids.iter().filter(|id| tenth.head(id)).count();
        assert!(
            (1_600..=2_400).contains(&kept),
            "kept {kept} of 20000 at 10 %"
        );
        let again = Sampler::from_values(Some("0.1"), Some("5"));
        assert!(ids.iter().all(|id| tenth.head(id) == again.head(id)));
        assert!(ids
            .iter()
            .all(|id| Sampler::from_values(Some("1"), None).head(id)));
        assert!(!ids
            .iter()
            .any(|id| Sampler::from_values(Some("0"), None).head(id)));
    }

    #[test]
    fn settings_are_read_clamped_and_defaulted() {
        assert_eq!(Sampler::from_values(None, None), Sampler::default());
        assert_eq!(Sampler::from_values(Some("7"), None).ratio, 1.0);
        assert_eq!(Sampler::from_values(Some("-1"), None).ratio, 0.0);
        assert_eq!(
            Sampler::from_values(Some("NaN"), None).ratio,
            DEFAULT_SAMPLE_RATIO
        );
        assert_eq!(
            Sampler::from_values(Some("x"), Some("y")),
            Sampler::default()
        );
        assert_eq!(
            Sampler::from_values(None, Some("250")).slow_nanos,
            250_000_000
        );
    }

    #[test]
    fn errors_and_slow_work_are_kept_without_sampling() {
        let sampler = Sampler::from_values(Some("0"), Some("100"));
        assert_eq!(sampler.keep(true, false, 0), Some(Keep::Sampled));
        assert_eq!(sampler.keep(false, true, 0), Some(Keep::Error));
        assert_eq!(sampler.keep(false, false, 100_000_000), Some(Keep::Slow));
        assert_eq!(sampler.keep(false, false, 99_999_999), None);
        assert_eq!(Keep::Error.kept_for(), Some("error"));
        assert_eq!(Keep::Sampled.kept_for(), None);
    }

    #[test]
    fn a_delivery_trace_travels_as_headers() {
        let sampler = Sampler {
            ratio: 1.0,
            ..Sampler::default()
        };
        let trace = DeliveryTrace::start(&sampler, 1_791_131_070_000_000_000);
        assert!(trace.context.sampled);
        let parent = trace.context.traceparent();
        let state = trace.tracestate();
        assert_eq!(state, "ih=c.1791131070000000000");
        assert_eq!(
            DeliveryTrace::from_headers(Some(&parent), Some(&state)),
            Some(trace.clone())
        );
        let foreign = format!("vendor=abc, {state}");
        assert_eq!(
            DeliveryTrace::from_headers(Some(&parent), Some(&foreign)),
            Some(trace.clone())
        );
        let unknown = DeliveryTrace::from_headers(Some(&parent), Some("ih=zz")).unwrap();
        assert_eq!(unknown.created_unix_nano, 0);
        assert_eq!(DeliveryTrace::from_headers(None, Some(&state)), None);
        assert_eq!(DeliveryTrace::from_headers(Some("bad"), None), None);
    }
}

/// The per-hop cost of tracing (run with `--release -- --ignored`):
/// parsing the incoming context, deciding, making a child and formatting
/// it, which every traced request pays once per hop.
#[cfg(test)]
mod cost {
    use super::*;

    #[test]
    #[ignore = "a measurement, run on demand"]
    fn per_hop_cost() {
        let sampler = Sampler::default();
        let value = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let rounds = 1_000_000u32;
        let started = std::time::Instant::now();
        let mut kept = 0u32;
        for round in 0..rounds {
            let parent = TraceContext::parse(std::hint::black_box(value)).unwrap();
            let child = parent.child();
            kept += u32::from(sampler.keep(child.sampled, round % 97 == 0, u64::from(round)).is_some());
            std::hint::black_box(child.traceparent());
        }
        let per = started.elapsed().as_nanos() as f64 / f64::from(rounds);
        let started = std::time::Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(TraceContext::root(&sampler));
        }
        let root = started.elapsed().as_nanos() as f64 / f64::from(rounds);
        println!("parse+child+keep+format: {per:.0} ns per hop; new root: {root:.0} ns ({kept} kept)");
    }
}
