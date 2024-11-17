# Contributing to IH-Muse

Thank you for your interest in contributing to IH-Muse! At this moment, the project is in its initial development phase and is not yet open for external contributions. We are diligently working towards a Minimum Viable Product (MVP) and establishing a stable foundation.

Once the project reaches a stage where community contributions can be integrated, this guide will be updated with detailed instructions on the contribution process. For now, we invite you to watch the repository for updates and participate in issue discussions.

We appreciate your understanding and look forward to your contributions in the future!

Best regards,

The IH-Muse Team

---

## Setting Up the Development Environment

To contribute to IH-Muse once it's open for contributions, you can prepare your development environment by following these typical steps:

1. **Fork the Repository**: Start by forking the IH-Muse repository (<https://github.com/your-username/ih-muse>) on GitHub to your own account.

2. **Clone the Fork**: Clone your fork to your local machine.

   ```bash
   git clone https://github.com/your-username/ih-muse.git
   cd ih-muse
   ```

3. **Install Dependencies**:

   - **Docker**: Make sure you have Docker installed on your system as it may be used for running services such as databases or other dependencies.

     - Docker Installation: Visit <https://docs.docker.com/get-docker/>

   - **Poetry**: IH-Muse uses Poetry for dependency management. Install Poetry using the recommended method from the official documentation at <https://python-poetry.org/docs/#installation>.

4. **Set Up the Project**: Inside the project directory, set up your local development environment using Poetry. This will install all dependencies, including those needed for development.

   ```bash
   poetry install
   ```

5. **Install Pre-commit Hooks**: After installing all dependencies, set up pre-commit hooks in your local repository. This ensures that code quality checks are automatically performed before each commit.

   ```bash
   poetry run pre-commit install
   ```

6. **Activate the Virtual Environment**: Use Poetry to activate the virtual environment.

   ```bash
   poetry shell
   ```

7. **Start Development**: You are now ready to start exploring the codebase. While we are not accepting contributions yet, you can familiarize yourself with the project structure and code.

   ```{attention}
   The Python docstrings will be rendered by MyST and `autodoc2`. Use Markdown to document them and any MyST formatting allowed. For examples, check the [MyST syntax cheat sheet](https://jupyterbook.org/en/stable/reference/cheatsheet.html), [Roles and Directives](https://myst-parser.readthedocs.io/en/latest/syntax/roles-and-directives.html), and the complete directives references at [MyST docs](https://mystmd.org/guide/directives).
   ```

---

## Running Tests and Checking Coverage

IH-Muse aims to maintain a high standard of code quality, which includes thorough testing and maintaining good test coverage. Here’s how you can run tests and check coverage:

1. **Running Tests**: After setting up your development environment, you can run the tests to ensure everything is working as expected.

   ```bash
   poetry run pytest
   ```

   This command will execute all the tests in the `tests` directory.

2. **Checking Test Coverage**: To check how much of the code is covered by tests, use the `coverage` tool.

   - First, run the tests with coverage tracking:

     ```bash
     poetry run coverage run -m pytest
     ```

   - Then, generate a coverage report. There are two ways to view the coverage report:

     - For a summary in the console, use:

       ```bash
       poetry run coverage report
       ```

     - For a more detailed HTML report, use:

       ```bash
       poetry run coverage html
       ```

       This will generate a report in the `htmlcov` directory. You can open `htmlcov/index.html` in a web browser to view it.

Please aim to maintain or improve the test coverage with your contributions. It’s recommended to add tests for any new code or when fixing bugs.

---

## License

Any contributions you make to this project will fall under the [MIT License](https://github.com/your-username/ih-muse/blob/main/LICENSE) that covers the IH-Muse project.

---

```{toctree}
:hidden:
:maxdepth: 2
:caption: Detailed Guides

ide
test
```
