// tests/it/test_replay.rs

use tokio::time::Duration;

use super::common::{client_type_from_env, TestContext, DEFAULT_WAIT_TIME};
use ih_muse::prelude::*;

#[tokio::test]
async fn test_record_and_replay_with_timestamps() {
    let client_type = client_type_from_env();
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let record_path = temp_dir.path().join("test_recording.json");

    // Create the TestContext with the custom configuration
    let ctx = TestContext::new_recording(None, record_path.to_str().unwrap().to_string()).await;

    // Register an element
    let local_elem_id = ctx.register_test_element().await;

    // Record metrics with delays
    for (i, value) in [42.0, 43.0, 44.0].iter().enumerate() {
        ctx.muse
            .send_metric(local_elem_id, "cpu_usage", *value)
            .await
            .expect("Failed to send metric");
        let node_resolution = ctx.muse.get_state().get_finest_resolution();
        // Wait enough time (min 2 times finest_resolution) to flush metrics to Poet client
        for _ in 0..i + 2 {
            tokio::time::sleep(node_resolution.to_duration()).await;
        }
    }

    // On case of poet check existing metrics before replaying (multiple tests may write)
    let mut previous_metrics = 0;
    if client_type == ClientType::Poet {
        let state = ctx.muse.get_state();
        let element_id = state
            .get_element_id(&local_elem_id)
            .expect("Element was not registered");
        // Retrieve metrics from the Mock client
        let poet_client = ctx.muse.get_client();
        let query = MetricQuery {
            start_time: None,
            end_time: None,
            element_id: Some(element_id),
            parent_id: None,
            metric_id: None,
        };

        previous_metrics = poet_client
            .get_metrics(&query, None)
            .await
            .expect("Failed to get metrics")
            .len();
    }

    // Drop the Muse, should flush and close the recorder
    drop(ctx);

    // Read and print the contents of the recording file
    let file_contents =
        std::fs::read_to_string(&record_path).expect("Failed to read recording file");
    println!("Recording file contents:\n{}", file_contents);
    assert_eq!(
        file_contents.lines().count(),
        5,
        "Recording file should have 5 events: config, reg elem and 3*metrics"
    );

    // Create a new Muse instance from the recorded config
    let replay_muse = Muse::check_and_replay(&record_path)
        .await
        .expect("Failed to create Muse and replay");

    // Verify that the replayed metrics are present in the Mock client's state
    // Retrieve the remote element ID
    let state = replay_muse.get_state();
    let element_id = state
        .get_element_id(&local_elem_id)
        .expect("Element was not registered");

    // Wait for the metrics to be processed
    tokio::time::sleep(DEFAULT_WAIT_TIME).await;

    // Retrieve metrics from the Mock client
    let poet_client = replay_muse.get_client();
    let query = MetricQuery {
        start_time: None,
        end_time: None,
        element_id: Some(element_id),
        parent_id: None,
        metric_id: None,
    };

    let metrics = poet_client
        .get_metrics(&query, None)
        .await
        .expect("Failed to get metrics");

    // If the client is Mock, we can create a new Muse instance and replay the events
    if client_type == ClientType::Mock {
        // Assert that all metrics were replayed
        assert_eq!(
            metrics.len(),
            3,
            "Expected 3 metrics to be replayed on a new Mock instance"
        );

        // Verify replayed metric values
        let replayed_values: Vec<Option<f32>> =
            metrics.into_iter().flat_map(|m| m.values).collect();
        assert_eq!(
            replayed_values,
            vec![Some(42.0), Some(43.0), Some(44.0)],
            "Replay values did not match recorded metrics"
        );
    } else {
        // Assert that all metrics were replayed
        assert_eq!(
            metrics.len(),
            previous_metrics + 3,
            "Expected 3 new metrics to be replayed from existing ones"
        );
    }
}

#[tokio::test]
async fn test_check_and_replay() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let replay_path = temp_dir.path().join("test_replay.json");

    // Create a replay file with a ConfigUpdate event
    {
        let mut recorder = FileRecorder::new(&replay_path).expect("Failed to create FileRecorder");

        let config = Config::new(
            vec!["http://localhost:8000".to_string()],
            ClientType::Poet,
            true, // Initially enabled recording
            Some("recording.json".to_string()),
            Some(Duration::from_millis(1)),
            TimestampResolution::Milliseconds,
            vec![ElementKindRegistration::new(
                "kind_code",
                None,
                "kind_name",
                "desc",
            )],
            vec![MetricDefinition::new("metric_code", "metric_name", "desc")],
            Some(Duration::from_secs(60)),
            3,
        )
        .expect("Failed to create config");

        recorder
            .record(RecordedEventWithTime {
                timestamp: utc_now_i64(),
                event: RecordedEvent::MuseConfig(config.clone()),
            })
            .await
            .expect("Failed to record ConfigUpdate");

        recorder.close().await.expect("Failed to close recorder");
    }

    // Check and replay logic
    Muse::check_and_replay(&replay_path)
        .await
        .expect("Replay failed");

    // Ensure the replay file exists
    assert!(replay_path.exists(), "Replay file does not exist");

    // Verify that the replayed config matches expectations
    let mut replayer = FileReplayer::new(&replay_path).expect("Failed to create FileReplayer");
    let mut replayed_config: Option<Config> = None;

    while let Some(event) = replayer
        .next_event()
        .await
        .expect("Failed to read replay event")
    {
        if let RecordedEvent::MuseConfig(config) = event.event {
            replayed_config = Some(config);
            break;
        }
    }

    let replayed_config = replayed_config.expect("No ConfigUpdate found in replay file");
    assert_eq!(
        replayed_config.endpoints,
        vec!["http://localhost:8000"],
        "Replay config endpoints do not match"
    );
    assert_eq!(
        replayed_config.client_type,
        ClientType::Poet,
        "Replay config client_type does not match"
    );
    assert_eq!(
        replayed_config.default_resolution,
        TimestampResolution::Milliseconds,
        "Replay config resolution does not match"
    );
    assert_eq!(
        replayed_config.element_kinds.len(),
        1,
        "Replay config element_kinds count does not match"
    );
    assert_eq!(
        replayed_config.metric_definitions.len(),
        1,
        "Replay config metric_definitions count does not match"
    );
}

#[tokio::test]
async fn test_initialize_with_config_recording() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let record_path = temp_dir.path().join("config_recording.json");

    // Create a configuration
    let config = Config::new(
        vec!["http://localhost:8000".to_string()],
        ClientType::Poet,
        true, // Enable recording
        Some(record_path.to_str().unwrap().to_string()),
        Some(Duration::from_millis(1)),
        TimestampResolution::Milliseconds,
        vec![ElementKindRegistration::new(
            "kind_code",
            None,
            "kind_name",
            "desc",
        )],
        vec![MetricDefinition::new("metric_code", "metric_name", "desc")],
        Some(Duration::from_secs(60)),
        3,
    )
    .expect("Failed to create config");

    // Initialize Muse
    let mut muse = Muse::new(&config).expect("Failed to create Muse");
    muse.initialize(None)
        .await
        .expect("Failed to initialize Muse");

    // Check if the configuration was recorded
    let mut replayer = FileReplayer::new(&record_path).expect("Failed to create FileReplayer");
    let mut found_config_event = false;

    while let Some(event) = replayer
        .next_event()
        .await
        .expect("Failed to read replay event")
    {
        if let RecordedEvent::MuseConfig(recorded_config) = event.event {
            found_config_event = true;
            assert_eq!(
                recorded_config, config,
                "Recorded config does not match original config"
            );
        }
    }

    assert!(found_config_event, "MuseConfig event was not recorded");
}
