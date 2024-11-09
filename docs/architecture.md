# Architecture Overview

The `ih-muse` project is composed of modular crates, each with distinct roles. Below is the flowchart representing the project structure and the data flow between components.

```mermaid
flowchart TB
    subgraph external["External Systems"]
        poet["Poet Server(s)"]
        cache["Cache Store"]
    end

    subgraph crates["Crate Structure"]
        core["ih-muse-core
        - Core types/traits
        - Data structures
        - Cache handling
        - Rate limiting"]

        client["ih-muse-client
        - API client
        - Server discovery
        - Request handling"]

        proto["ih-muse-proto
        - Data serialization
        - Protocol definitions"]

        test["ih-muse-test
        - Test helpers
        - Recording/Replay
        - Mock servers"]

        main["ih-muse
        - High-level interface
        - Feature coordination"]

        bindings["Language Bindings"]
        py["ih-muse-python"]
        other["(future: nodejs, etc)"]

        main --> core
        main --> client
        main --> proto
        client --> core
        client --> proto
        py --> main
        other --> main
        test --> core
        test --> client
    end

    main --> poet
    client --> poet
    core --> cache
```
