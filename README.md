# 🦀 Rusty Search Engine (WIP)

A high-performance, work-in-progress search engine built from scratch in Rust, inspired by solutions like Algolia and Typesense. Designed with Clean Architecture principles for maintainability and scalability.

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen?style=flat-square)](#) <!-- Replace # with actual build status link -->
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=flat-square)](https://opensource.org/licenses/MIT)
[![Rust Version](https://img.shields.io/badge/rust-1.78%2B-blue?style=flat-square)](https://www.rust-lang.org) <!-- Update Rust version if needed -->

## ✨ Features (Implemented)

- **Dynamic Collections:** Define schemas via JSON configuration.
- **Document Management:** Index single or batches of documents, delete documents.
- **Flexible Search:**
  - Keyword search (basic substring matching currently).
  - Advanced filtering (equality, numeric ranges `gte`/`lte`/`gt`/`lt`).
  - Multi-field sorting (`asc`/`desc`).
  - Pagination (`limit`/`offset`).
- **System Stats:** `/stats` endpoint for resource monitoring (RAM, Disk, Engine state).
- **Clean Architecture:** Separation of concerns (Domain, Application, Infrastructure, API).
- **Dockerized:** Easy setup and deployment using Docker and Docker Compose.
- **Async:** Built on Tokio and Axum for concurrent performance.
- **Logging:** Structured logging using Tracing.

## 🛠️ Tech Stack

- **Language:** Rust (Stable)
- **Web Framework:** Axum
- **Async Runtime:** Tokio
- **Logging:** Tracing
- **In-Memory Storage:** DashMap (for current prototype)
- **System Info:** Sysinfo
- **Containerization:** Docker / Docker Compose

## 🚀 Getting Started

1.  **Clone the repository:**
    ```bash
    git clone https://github.com/bernabedev/rust_search_engine.git
    cd rust_search_engine
    ```
2.  **Build and Run using Docker Compose:**

    ```bash
    docker compose up --build -d
    ```

    _(Ensure Docker Desktop or Docker Engine is running.)_

3.  **Verify:** The API should be available at `http://localhost:3000`. Check the health endpoint:
    ```bash
    curl http://localhost:3000/health
    ```
    Expected output: `OK`

## 🔌 API Usage

Interact with the engine via its REST API. Core concepts:

- **Collections:** Define the structure (schema) of your data. Manage via `/collections`.
- **Documents:** Add/delete data within collections via `/collections/{name}/documents`.
- **Search:** Query collections using `POST /collections/{name}/_search` with a JSON body for query, filters, sorting, and pagination.

For detailed examples of all available endpoints, refer to the `requests.http` file in the repository root (compatible with VS Code's REST Client extension or IntelliJ's HTTP Client).

## 🚧 Project Status

This project is currently **under active development** and serves as a learning platform for building a search engine in Rust with best practices.

**Current limitations:**

- Uses basic in-memory data structures (`DashMap`) for storage and indexing (not persistent).
- Search implementation is naive (substring matching) – needs a proper index (e.g., Tantivy).
- Authentication is not yet implemented.

## 🌱 Future Goals

- Integrate a real search library (e.g., Tantivy) for efficient indexing and searching.
- Implement persistent storage (e.g., RocksDB, Sled).
- Add API Key Authentication & Authorization.
- Implement Faceting.
- Implement Autocomplete / Typeahead suggestions.
- Add Synonym support.
- Improve Relevance Ranking.
- Explore clustering and distributed deployment options.
- Set up CI/CD pipelines.

## 🤝 Contributing

Contributions, issues, and feature requests are welcome! Feel free to check the [issues page](https://github.com/bernabedev/rust_search_engine/issues) or open a Pull Request.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
