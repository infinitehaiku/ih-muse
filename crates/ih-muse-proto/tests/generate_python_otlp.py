#!/usr/bin/env python3
"""Generate OTLP/HTTP-Protobuf wire fixtures with the pinned Python SDK."""

from __future__ import annotations

import hashlib
import importlib.metadata
import json
import logging
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from opentelemetry import metrics, trace
from opentelemetry.exporter.otlp.proto.http._log_exporter import OTLPLogExporter
from opentelemetry.exporter.otlp.proto.http.metric_exporter import OTLPMetricExporter
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk._logs import LoggerProvider, LoggingHandler
from opentelemetry.sdk._logs.export import SimpleLogRecordProcessor
from opentelemetry.sdk.metrics import MeterProvider
from opentelemetry.sdk.metrics.export import PeriodicExportingMetricReader
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor


class CaptureHandler(BaseHTTPRequestHandler):
    """Captures authenticated protobuf requests and returns an empty OTLP success."""

    captures: dict[str, bytes] = {}

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        assert self.headers.get("Authorization") == "Bearer fixture-token"
        length = int(self.headers["Content-Length"])
        self.captures[self.path] = self.rfile.read(length)
        self.send_response(200)
        self.send_header("Content-Type", "application/x-protobuf")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, _format: str, *_args: object) -> None:
        pass


def main() -> None:
    output = Path(sys.argv[1])
    output.mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer(("127.0.0.1", 0), CaptureHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base = f"http://127.0.0.1:{server.server_port}"
    headers = {"Authorization": "Bearer fixture-token"}
    resource = Resource.create(
        {"service.name": "python-fixture", "service.instance.id": "fixture-1"}
    )

    tracer_provider = TracerProvider(resource=resource)
    tracer_provider.add_span_processor(
        SimpleSpanProcessor(
            OTLPSpanExporter(endpoint=f"{base}/v1/traces", headers=headers)
        )
    )
    tracer = tracer_provider.get_tracer("ih.fixture.python")
    with tracer.start_as_current_span("task attempt zero") as span:
        span.set_attribute("task.attempt", 0)
    tracer_provider.shutdown()

    metric_exporter = OTLPMetricExporter(endpoint=f"{base}/v1/metrics", headers=headers)
    metric_reader = PeriodicExportingMetricReader(metric_exporter)
    meter_provider = MeterProvider(resource=resource, metric_readers=[metric_reader])
    meter = meter_provider.get_meter("ih.fixture.python")
    meter.create_counter("task.attempt.completed", unit="{attempt}").add(
        1, {"task.attempt": 0}
    )
    meter_provider.force_flush()
    meter_provider.shutdown()

    logger_provider = LoggerProvider(resource=resource)
    logger_provider.add_log_record_processor(
        SimpleLogRecordProcessor(
            OTLPLogExporter(endpoint=f"{base}/v1/logs", headers=headers)
        )
    )
    handler = LoggingHandler(level=logging.NOTSET, logger_provider=logger_provider)
    logger = logging.getLogger("ih.fixture.python")
    logger.setLevel(logging.INFO)
    logger.addHandler(handler)
    logger.info("task attempt zero completed", extra={"task.attempt": 0})
    logger.removeHandler(handler)
    logger_provider.shutdown()
    server.shutdown()
    thread.join()

    files = {}
    for signal in ("metrics", "traces", "logs"):
        payload = CaptureHandler.captures[f"/v1/{signal}"]
        name = f"{signal}.bin"
        (output / name).write_bytes(payload)
        files[name] = {"bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}
    versions = {
        package: importlib.metadata.version(package)
        for package in (
            "opentelemetry-api",
            "opentelemetry-sdk",
            "opentelemetry-exporter-otlp-proto-http",
        )
    }
    manifest = {"generator": "python-otel-sdk", "versions": versions, "files": files}
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
