# Changelog

All notable changes to Super Pull Requests will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] - 2026-02-22

- Base child PRs on parent PR branches in Github allowing auto-rebase
- Remove land & close & list.
- Add commenting feature that makes PR chains obvious
- Rename `diff` to `push`
- Rename `amend` to `fetch`
- Rename `patch` to `adopt`
- Optimize features for dependent stack style development

## [0.1.0] - 2025-11-15

### Added
- Initial release of Super Pull Requests (SPR)
- Power tool for Jujutsu + GitHub workflows
- Amend-friendly single PR workflow: Amend freely in jj, review cleanly on GitHub
- Effortless stacked PR support: Independent or dependent changes with automatic rebase handling
- Change-based workflow using Jujutsu's stable change IDs
- Commands: `diff`, `land`, `list`, `close`, `amend`
- Cherry-pick mode for independent changes
- Automatic PR updates without force-push confusion
- Support for both single PRs and stacked PRs
- GitHub API integration via REST and GraphQL
- Comprehensive documentation and guides

### Changed
- Rebranded from "jj-spr (Jujutsu Stacked Pull Requests)" to "Super Pull Requests"
- Version reset to 0.1.0 for official release
- Updated project metadata and repository information
