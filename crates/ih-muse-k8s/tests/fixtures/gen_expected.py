"""Regenerates expected-*.json: runs the reference Python Muse's
collect_k8s_graph (ih-infra scripts/k8s_muse.py, only imported) on the
recorded API answers in this directory.

    python3 gen_expected.py <ih-infra>/scripts
"""
import json, sys
from pathlib import Path
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
import k8s_muse

FIX = Path(__file__).resolve().parent
NS = "infinite-haiku-p2"
ROUTES = {
    "/api/v1/nodes": "nodes.json",
    "/apis/metrics.k8s.io/v1beta1/nodes": "node-metrics.json",
    f"/api/v1/namespaces/{NS}/pods": "pods.json",
    f"/apis/metrics.k8s.io/v1beta1/namespaces/{NS}/pods": "pod-metrics.json",
    f"/api/v1/namespaces/{NS}": "namespace.json",
}
NOW = 1_791_108_000_000_000_000  # 2026-10-04T10:00:00Z
ORG = "ih-local"
CLUSTER = "5f51b1b5-8445-47c0-a9c7-222bb2c92398"

class Fake:
    def __init__(self, metrics=True):
        self.metrics = metrics
    def get(self, path):
        if not self.metrics and "metrics.k8s.io" in path:
            raise RuntimeError("HTTP 503 metrics-server unavailable")
        return json.loads((FIX / ROUTES[path]).read_text())

full = Fake()
ns_uid = k8s_muse.resolve_namespace_uid(full, NS)
cases = {
    "expected-full.json": k8s_muse.collect_k8s_graph(full, NS, ORG, CLUSTER, NOW, 5_000_000_000, lambda m: None, namespace_uid=ns_uid),
    "expected-no-metrics.json": k8s_muse.collect_k8s_graph(Fake(False), NS, ORG, CLUSTER, NOW, 5_000_000_000, lambda m: None, namespace_uid=None),
}
for name, batch in cases.items():
    (FIX / name).write_text(json.dumps(batch, indent=1) + "\n")
    print(name, len(batch["entities"]), "entities", len(batch["relations"]), "relations", len(batch["observations"]), "observations", len(batch["availability"]), "availability")
