# oxIDIZER

![oxIDIZER Logo](images/oxIDIZER_logo.png)

**oxIDIZER** is a powerful collection of modular Rust libraries designed to work together or independently. It provides a robust foundation for building data-driven applications, web services, and automated file processing systems.

## Project Vision

Modern software requires flexibility. oxIDIZER (pronounced "oxidizer") follows a "pick-and-choose" philosophy. You can use the core [Generic Data Object (GDO)](./ox_data_object) for dynamic data management, or pull in the [Webservice Framework](./ox_webservice) for building extensible APIs, all while benefiting from shared infrastructure like [ox_fileproc](./ox_fileproc) for surgical configuration handling.

---

## Workspace Components

The toolkit is organized into logical layers:

### 🧩 Core Data & Messaging
*   **[ox_data_object](./ox_data_object)**: The foundation of oxIDIZER. A flexible, event-driven data object system with type-safety and automatic conversion.
*   **[ox_data_broker](./ox_data_broker)**: Manages data flow and distribution between components.
*   **[ox_event_bus](./ox_event_bus)**: A high-performance pub/sub system supporting local and MQTT-based events.
*   **[ox_messaging_client](./ox_messaging_client)**: Standardized client interfaces for messaging protocols.
*   **[ox_callback_manager](./ox_callback_manager)**: Decoupled logic registration and execution.
*   **[ox_locking](./ox_locking)**: Cross-module synchronization and resource locking.

### 💾 Persistence Layer
*   **[ox_persistence](./ox_persistence)**: Core traits and API for consistent data storage across drivers.
*   **[ox_persistence_driver_manager](./ox_persistence_driver_manager)**: Manages dynamic loading and lifecycle of persistence drivers.
*   **[ox_persistence_datasource_manager](./ox_persistence_datasource_manager)**: Configuration and management for databases and file-based data sources.
*   **Drivers**: Modular support for **PostgreSQL**, **MySQL**, **MSSQL**, **SQLite**, as well as **YAML**, **JSON**, **XML**, and **Delimited** files.

### ⚙️ Infrastructure & Processing
*   **[ox_fileproc](./ox_fileproc)**: Advanced recursive configuration loading with variable substitution and structure-aware "surgical" file editing.
*   **[ox_pipeline](./ox_pipeline)**: A flexible execution engine for data processing pipelines.
*   **[ox_type_converter](./ox_type_converter)**: The unified type conversion engine used throughout the toolkit.
*   **[ox_package_manager](./ox_package_manager)**: Handles dynamic crate loading and dependency management.

### 🌐 Web & UI
*   **[ox_webservice](./ox_webservice)**: A modular, plugin-based web server framework.
*   **[ox_forms](./ox_forms)**: Generic form definitions and rendering engine.
*   **[ox_forms_server](./ox_forms_server)**: Server-side handling and serving of dynamic forms.
*   **[ox_forms_client](./ox_forms_client)**: WASM-based client-side form logic.

---

## Getting Started

Each crate is located in its own directory and can be used as a standard Cargo dependency.

### Basic Development Flow
1. **Clone the repository**:
    ```bash
    git clone https://github.com/justlikeef/oxIDIZER
    ```
2. **Run tests for the whole workspace**:
    ```bash
    cargo test
    ```
3. **Build specific components**:
    ```bash
    cargo build -p ox_data_object
    ```

## Documentation

*   Each crate contains its own `README.md` for specific usage details.
*   Architecture overviews and schemas can be found in the [docs/](./docs) folder.

## License

This project is licensed under the MIT License - see the [LICENSE](./LICENSE) file for details.
