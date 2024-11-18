# {py:mod}`ih_muse.muse.muse`

```{py:module} ih_muse.muse.muse
```

```{autodoc2-docstring} ih_muse.muse.muse
:allowtitles:
```

## Module Contents

### Classes

````{list-table}
:class: autosummary longtable
:align: left

* - {py:obj}`Muse <ih_muse.muse.muse.Muse>`
  - ```{autodoc2-docstring} ih_muse.muse.muse.Muse
    :summary:
    ```
````

### API

`````{py:class} Muse(config: ih_muse.config.Config)
:canonical: ih_muse.muse.muse.Muse

```{autodoc2-docstring} ih_muse.muse.muse.Muse
```

```{rubric} Initialization
```

```{autodoc2-docstring} ih_muse.muse.muse.Muse.__init__
```

````{py:attribute} _muse
:canonical: ih_muse.muse.muse.Muse._muse
:type: ih_muse.ih_muse.PyMuse
:value: >
   None

```{autodoc2-docstring} ih_muse.muse.muse.Muse._muse
```

````

````{py:method} initialize(timeout: float | None = None) -> None
:canonical: ih_muse.muse.muse.Muse.initialize
:async:

```{autodoc2-docstring} ih_muse.muse.muse.Muse.initialize
```

````

````{py:method} create(config: ih_muse.config.Config, timeout: float | None = None) -> ih_muse.muse.muse.Muse
:canonical: ih_muse.muse.muse.Muse.create
:async:
:classmethod:

```{autodoc2-docstring} ih_muse.muse.muse.Muse.create
```

````

````{py:method} is_initialized() -> bool
:canonical: ih_muse.muse.muse.Muse.is_initialized

```{autodoc2-docstring} ih_muse.muse.muse.Muse.is_initialized
```

````

````{py:method} register_element(kind_code: str, name: str, metadata: dict[str, str], parent_id: int | None = None) -> int
:canonical: ih_muse.muse.muse.Muse.register_element
:async:

```{autodoc2-docstring} ih_muse.muse.muse.Muse.register_element
```

````

````{py:method} send_metric(local_elem_id: int, metric_code: str, value: float) -> None
:canonical: ih_muse.muse.muse.Muse.send_metric
:async:

```{autodoc2-docstring} ih_muse.muse.muse.Muse.send_metric
```

````

````{py:method} replay(replay_path: str) -> None
:canonical: ih_muse.muse.muse.Muse.replay
:async:

```{autodoc2-docstring} ih_muse.muse.muse.Muse.replay
```

````

`````
