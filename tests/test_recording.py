from ih_muse import Muse, recording_context


class K8sMetricCollector:
    def __init__(self):
        self.muse = Muse()

    def collect_metrics(self):
        # Normal metric collection
        with recording_context(output_file="k8s_session.json"):
            self.muse.send_metric("node1", "cpu_usage", 75.5)
            self.muse.send_metric("node1", "memory_usage", 60.3)
            # More metric collection...


# In test environment
def test_k8s_metrics():
    # Replay the recorded session
    replay_session("k8s_session.json")
