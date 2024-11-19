# Contributing to `ih-muse`

Contributions are welcome, and they are greatly appreciated!
Every little bit helps, and credit will always be given.

You can contribute in many ways:

## Types of Contributions

### Report Bugs

Report bugs at <https://github.com/infinitehaiku/ih-muse/issues>

If you are reporting a bug, please include:

- Your operating system name and version.
- Any details about your local setup that might be helpful in troubleshooting.
- Detailed steps to reproduce the bug.

### Fix Bugs

Look through the GitHub issues for bugs.
Anything tagged with "bug" and "help wanted" is open to whoever wants to implement a fix for it.

### Implement Features

Look through the GitHub issues for features.
Anything tagged with "enhancement" and "help wanted" is open to whoever wants to implement it.

### Write Documentation

`ih-muse` could always use more documentation, whether as part of the official docs or in docstrings.

### Submit Feedback

The best way to send feedback is to file an issue at <https://github.com/infinitehaiku/ih-muse/issues>.

If you are proposing a new feature:

- Explain in detail how it would work.
- Keep the scope as narrow as possible to make it easier to implement.

---

## Get Started

Ready to contribute? Here's how to set up `ih-muse` for local development.
Please note this documentation assumes you already have `uv`, `make`, and `Git` installed and ready to go.

1. **Fork the `ih-muse` repo on GitHub.**

2. **Clone your fork locally:**

   ```bash
   cd <directory_in_which_repo_should_be_created>
   git clone git@github.com:YOUR_NAME/ih-muse.git
   ```

3. **Navigate into the directory:**

   ```bash
   cd ih-muse
   ```

4. **Install dependencies and pre-commit hooks using `make`:**

   ```bash
   make install
   ```

5. **Create a branch for local development:**

   ```bash
   git checkout -b name-of-your-bugfix-or-feature
   ```

   Now you can make your changes locally.

6. **Testing `ClientType`:**
   By default, the tests will run using the `Mock` client. If you want to test with the `Poet` client, which requires a running Poet instance, you need to set the `IH_MUSE_CLIENT_TYPE` environment variable to `Poet`.

   **Running tests with `Mock` (default):**

   ```bash
   make test
   ```

   **Running integration tests with `Poet`:**

   ```bash
   export IH_MUSE_CLIENT_TYPE=Poet
   make test
   ```

   or

   ```bash
   make test-integration
   ```

7. **Add test cases for your changes:**
   Add all test cases to the `tests` directory.

8. **Run formatting and linting checks:**

   ```bash
   make check
   ```

9. **Validate all unit tests pass:**

   ```bash
   make test
   ```

10. **Run `tox` to test across multiple Python versions:**

    ```bash
    tox
    ```

    _This step requires multiple Python versions installed. It is optional as it runs automatically in the CI/CD pipeline._

11. **Commit your changes and push your branch to GitHub:**

    ```bash
    git add .
    git commit -m "Your detailed description of your changes."
    git push origin name-of-your-bugfix-or-feature
    ```

12. **Submit a pull request on GitHub.**

---

## Pull Request Guidelines

Before you submit a pull request, check that it meets these guidelines:

1. The pull request should include tests.

2. If the pull request adds functionality, ensure that the docs are updated.
   Add a docstring for new functions and update the feature list in the `README.md`.
