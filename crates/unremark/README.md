# Unremark

Unremark is a Rust library for analyzing and removing redundant comments from code.

### Development with Python bindings

- Build the package: `maturin build --features python`
- Run tests: `maturin develop && cargo test --features python`
- Install in editable mode: `maturin develop --features python`

### Requirements

- Python >= 3.8
- Rust toolchain
- `cffi` Python package
