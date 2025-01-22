# unremark

A tool to find comments and remove redundant comments in code.


## Usage
```
cargo run examples/python --fix
```

## Development

Install program locally:
```
cargo install --path .
```
This is useful for testing the program locally in other projects.

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
