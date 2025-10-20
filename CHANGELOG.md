# Changelog

## [Unreleased]

### Changed

* **refactor:** Separated diff and kiro functionality into dedicated modules for better maintainability
* **config:** Introduced structured configuration with `[kiro]` and `[diff]` sections in config.toml
* **architecture:** Created library structure with `agpod::diff`, `agpod::kiro`, and `agpod::config` modules
* **docs:** Updated README with configuration examples and architecture overview

### Added

* **lib:** Added `src/lib.rs` to enable library usage of agpod modules
* **config:** New `DiffConfig` structure supporting diff-specific settings (output_dir, thresholds)
* **config:** Added `version` field to configuration for tracking schema changes and enabling future deprecation warnings
* **config:** Added `XDG_CONFIG_HOME` environment variable support for flexible configuration directory location
* **tests:** Comprehensive test suite for new configuration module (47 tests total)

### BREAKING CHANGES

* **config:** Configuration file format has changed. The old flat format is no longer supported. All configuration must now use structured sections `[kiro]` and `[diff]` with a required `version` field. Users must migrate their `~/.config/agpod/config.toml` and `.agpod.toml` files to the new format. See examples/config.toml for the new format.

## [0.4.1](https://github.com/towry/agpod/compare/v0.4.0...v0.4.1) (2025-10-14)


### Bug Fixes

* **plugin:** rename plugin from "branch_name" to "name" and exclude install.sh from releases ([#38](https://github.com/towry/agpod/issues/38)) ([c6d44d2](https://github.com/towry/agpod/commit/c6d44d297f0cf42da712ec6907de35e2227c0992))

## [0.4.0](https://github.com/towry/agpod/compare/v0.3.0...v0.4.0) (2025-10-14)


### Features

* Add kiro workflow subcommand for PR draft management ([#21](https://github.com/towry/agpod/issues/21)) ([ae95ba4](https://github.com/towry/agpod/commit/ae95ba451c7a125c95eaa74cbb328dc21bb8b139))
* **kiro:** add fuzzy filter enhancements and auto-detection to pr command ([#29](https://github.com/towry/agpod/issues/29)) ([9bdd057](https://github.com/towry/agpod/commit/9bdd05702f046ea22fdf690922404f77eefa57d7))
* save REVIEW.md in chunks directory alongside diff files ([#15](https://github.com/towry/agpod/issues/15)) ([43441a3](https://github.com/towry/agpod/commit/43441a3d91b6ab6979c8928f580e0d2989e51751))


### Bug Fixes

* **workflow:** remove invalid package-name parameter from release-please action v4 ([#31](https://github.com/towry/agpod/issues/31)) ([be348f0](https://github.com/towry/agpod/commit/be348f044cde818c1e451189dc17cbef54f4d5b0))
* **workflow:** replace deprecated command usage with help command in pr-asset-build ([#33](https://github.com/towry/agpod/issues/33)) ([a7edf63](https://github.com/towry/agpod/commit/a7edf639c39e0a457dccb7a9993a06722f567ea9))

## [0.3.0](https://github.com/towry/agpod/compare/v0.2.0...v0.3.0) (2025-10-13)


### Features

* **save:** add project-specific folders, REVIEW.md generation with intelligent updates, machine-readable output, help options, and path expansion ([#13](https://github.com/towry/agpod/issues/13)) ([c9391ba](https://github.com/towry/agpod/commit/c9391ba8b0f8f8a5a339a6adaf6c943b5811ac4d))


### Bug Fixes

* **install:** auto-upgrade existing binary without prompting ([#9](https://github.com/towry/agpod/issues/9)) ([9590547](https://github.com/towry/agpod/commit/95905472a40a2f4cfc750b7c88928d4977f14885))
