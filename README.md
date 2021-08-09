# p-vector-rs
<!--
Maintain your own .deb repository now!
Scanning packages, generating `Packages`, `Contents-*` and `Release`, all in one single binary.

Multi repository, finding potential file collisions, checking shared object compatibilities and more integrity checking features is coming.
-->

## Build Dependencies
- Rust 1.51+
- Clang/LLVM
- `pkg-config`

## Runtime Dependencies
- OpenSSL (libcrypto) (`libssl-dev` in Debian 10)
- LibLZMA (`liblzma-dev` in Debian 10)
- Nettle (`nettle-dev` in Debian 10)

And you need a PostgreSQL server. You will need to deploy one on your device.

## Building Instructions
1. Install both build and runtime dependencies
2. Spin up the PostgreSQL server, it's recommended to create a separate database like this: `createdb <database_name>`
3. Type `cargo install sqlx-cli`
4. Execute `export DATABASE_URL="postgres://localhost/<database_name>"`
5. Execute `sqlx migrate run`
6. Run `cargo build --release`

PostgreSQL server is required when building because the compiler will check if the SQL statements are semantically correct.

## Setup Instructions
1. Download or build `p-vector`
2. You will need a signing key for your repository, you can generate a new one using `gpg --gen-key`
3. Open `config.toml` in your favorite text editor and adapt the configurations to your needs
4. Save the configuration to somewhere (e.g. `/etc/p-vector/config.toml`)
5. Start the PostgreSQL server if not already done so
6. Run `p-vector --config <path/to/config.toml> full` to start the first scan
