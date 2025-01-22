# unremark

A tool to find comments and remove redundant comments in code.


## Usage

Check the code of a single file:
```
cargo run examples/example.py
```

Check the code of a directory:
```
cargo run examples
```

Fix the code of a single file:
```
cargo run examples/example.py --fix
```

Fix the code of a directory:
```
cargo run examples --fix
```

Output in JSON format:
```
cargo run examples --json
```

Output in JSON format with fix:
```
cargo run examples --json --fix
```

## Development

Prerequisites:
- Rust
- Cargo
- OpenAI API key

To build the program:
```
cargo build
```

To run the program after making changes:
```
cargo run
```

## To use the program in other projects without building it

Install program locally:
```
cargo install --path .
```

Now you can run the program with `unremark` in your shell.
```
unremark examples/example.py
```

This is useful for testing the program locally in other projects without having to build and release a new version of the program.

Note: be sure to create a `.env` file in the root of the project with the `OPENAI_API_KEY` and `OPENAI_API_MODEL` environment variables. Or set the environment variables in your shell.


## TODO
- [x] Add support for javascript
- [x] Add support for typescript
- [x] Add support for python
- [x] Add support for json output
- [ ] Add support for rust
- [ ] Add support for ignoring specific files
- [ ] Add support for ignoring specific lines
- [ ] Add support for ignoring specific comments (e.g. `# noqa: E501` `# TODO` `# FIXME`)
