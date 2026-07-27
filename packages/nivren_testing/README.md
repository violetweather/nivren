# nivren_testing

Typed assertions plus explicit channel-backed scheduling gates for reproducible concurrency tests. Assertions return `Result<Null,String>`, so suites preserve ordinary `or give` control flow. `gate`, `open`, `pass`, and `checkpoint` let tests release workers and observe milestones in a chosen order without sleeps or timing guesses; ordinary `Task` and `Channel` capabilities remain visible.
