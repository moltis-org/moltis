# Pipelight Schema Generator

This tool is a Rust program wrapped in a Nix flake that processes a list of file paths related to 'pipelight' jobs. It generates various schema formats (JSON-LD, CBOR, DASL-like, RDFa HTML) and a Buildbot Nix configuration based on these file paths, and also provides an analysis of common naming patterns found within the paths.

## Table of Contents

- [Features](#features)
- [Setup and Installation](#setup-and-installation)
- [Usage](#usage)
- [Generated Outputs](#generated-outputs)
- [Integration with Buildbot Nix](#integration-with-buildbot-nix)

## Features

-   **File Path Processing**: Reads a list of file paths from a specified input file.
-   **Schema Generation**:
    -   **JSON-LD**: Generates a JSON-LD schema representing the job data, using `schema.org` vocabulary.
    -   **CBOR**: Produces a CBOR (Concise Binary Object Representation) serialization of the job data.
    -   **DASL-like**: Provides a human-readable, DASL-like (Data Analysis and Specification Language) schema definition for the processed data structure.
    -   **RDFa HTML**: Generates an HTML snippet with embedded RDFa annotations for semantic web integration.
-   **Pattern Analysis**: Identifies and reports common extensions, filenames, and parent directory patterns in the input file paths.
-   **Buildbot Nix Configuration**: Generates a Nix expression that defines a Buildbot factory and individual shell steps for each file path, demonstrating integration capabilities.

## Setup and Installation

This project uses [Nix](https://nixos.org/) for a reproducible development environment.

1.  **Clone the repository**:
    ```bash
    git clone <repository_url>
    cd moltis/pipelight-schema-generator
    ```

2.  **Ensure Nix is installed**:
    If you don't have Nix installed, follow the instructions on [nixos.org](https://nixos.org/download.html).

3.  **Enter the development environment**:
    The project provides a `flake.nix` file to define its development environment.
    ```bash
    nix develop
    ```
    This command will set up a shell with all the necessary dependencies (Rust toolchain, etc.) for the project.

## Usage

Once you are in the Nix development environment (`nix develop`), you can use the provided `Makefile` to run the schema generator.

1.  **Prepare your input file**:
    The tool expects an input file named `data-locate-pipelight-shell-feb-28.txt` in the parent directory of `pipelight-schema-generator/`. This file should contain one file path per line.

2.  **Run the generator**:
    ```bash
    make run
    ```
    This command will compile and execute the Rust program. The output will be printed directly to your console, including:
    -   Current working directory.
    -   JSON-LD Schema.
    -   CBOR Data (Hex encoded).
    -   CBOR-DASL Schema Definition (Textual).
    -   RDFa HTML Snippet.
    -   Common Name Patterns Analysis (Extensions, Filenames, Parent Directories).
    -   Buildbot Nix Configuration.

## Generated Outputs

The tool generates several types of outputs to cover different schema and integration needs:

### JSON-LD Schema
A JSON-LD (JavaScript Object Notation for Linked Data) representation of the processed file paths. This format is suitable for semantic web applications and data interoperability. It uses `https://schema.org` as a context.

### CBOR Data (Hex encoded)
A hexadecimal representation of the CBOR (Concise Binary Object Representation) encoded `JobData`. CBOR is a compact binary serialization format, often used in constrained environments.

### CBOR-DASL Schema Definition (Textual)
A textual description in a DASL-like (Data Analysis and Specification Language) format, outlining the structure of the `FilePath` and `JobData` types. This acts as a human-readable schema definition.

### RDFa HTML Snippet
An HTML snippet embedded with RDFa (Resource Description Framework in attributes) annotations. This allows embedding semantic metadata directly within HTML documents.

### Common Name Patterns Analysis
A breakdown of frequently occurring file extensions, filenames, and parent directory structures found in the input `data-locate-pipelight-shell-feb-28.txt`. This can help in understanding the nature of the processed 'pipelight' jobs.

### Buildbot Nix Configuration
A Nix expression (`.nix` file content) suitable for integration with `buildbot-nix`. This configuration defines:
-   A `pipelightJobsFactory` Buildbot factory.
-   Individual `Shell` steps within the factory, one for each file path, with a descriptive name and a command to "process" the file (currently an `echo` command placeholder).
-   A `pipelightJobBuilder` Buildbot builder that uses the `pipelightJobsFactory`.

This generated Nix configuration can be integrated into a larger NixOS system using `buildbot-nix` to define automated build and testing jobs for 'pipelight' related tasks.

## Integration with Buildbot Nix

The generated Buildbot Nix configuration is a standalone Nix expression. To integrate it into an existing `buildbot-nix` setup, you would typically import this generated expression into your `buildbot-nix`-managed NixOS configuration.

For example, in your `configuration.nix` or a specific Buildbot-related Nix file, you could have:

```nix
# In your NixOS configuration file (e.g., configuration.nix)
{ config, pkgs, lib, ... }:

{
  imports = [
    # ... other imports
    ./path/to/generated-pipelight-buildbot-config.nix
  ];

  services.buildbot-master.settings = {
    # ... existing buildbot master settings
    # The generated factories and builders will merge with these settings
  };

  # ... rest of your configuration
}
```

The generated configuration is designed to seamlessly extend the `services.buildbot-master.settings` attribute set. You would save the output of the `--- Buildbot Nix Configuration ---` section to a `.nix` file (e.g., `generated-pipelight-buildbot-config.nix`) and then import it as shown above.
