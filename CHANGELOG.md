# Changelog

All notable changes to this project will be documented in this file.

## [0.11.2] - 2026-02-21

### Added
- Added tests and refactored. ([8c68ae5](https://github.com/oxicrab/oxicrab/commit/8c68ae550b5500e21de62418564d5dffd90bc2a1))
- Added sandbox support on macos and macos CI deploy ([4c97dff](https://github.com/oxicrab/oxicrab/commit/4c97dffa97d6df9577bf1dbfc36023687564c43b))
- Task #35: fs2 shared locks on reads ([c718e00](https://github.com/oxicrab/oxicrab/commit/c718e0026399f58c43dc55615662cf9da77fd695))
- Task #31: Memory store daily notes fs2 locking (HIGH) ([e306d79](https://github.com/oxicrab/oxicrab/commit/e306d792e4bebef04d0c1decb8b2f9e70ce6c5e3))
- Task #26: Session manager fs2 locking (CRITICAL) ([5cd74f8](https://github.com/oxicrab/oxicrab/commit/5cd74f84868e9aca5a989b7cd02f46b4e6817565))
- Task #20: Sanitize agent loop and subagent tool execution errors ([0c630e4](https://github.com/oxicrab/oxicrab/commit/0c630e47cddb4fb10cdb9725b7876f742b90177d))
- Part 1: MCP Server Sandboxing ([0ee9d10](https://github.com/oxicrab/oxicrab/commit/0ee9d10fdba6748040e166fa980d7c177607bc9d))
- shell AST analysis and Landlock filesystem sandbox ([5293bd1](https://github.com/oxicrab/oxicrab/commit/5293bd166d02c1eb8bca5d5b621c4656a24fb336))
- Added pairing docs and updates ([e248c62](https://github.com/oxicrab/oxicrab/commit/e248c628123407d004a0040fd691a692071992b7))


### CI/CD
- Package validation ([aca7a4e](https://github.com/oxicrab/oxicrab/commit/aca7a4e6fcb4e326d4720a9ddad49665a01ca9e8))


### Changed
- Changed provider model ([b92217f](https://github.com/oxicrab/oxicrab/commit/b92217f00fd64a917f04d55ec30b08d3c90db882))
- Updated packaging and dependencies ([4719ad7](https://github.com/oxicrab/oxicrab/commit/4719ad7e7bca28af25c85a95cc9239aca015be80))
- Extracted tests ([66d41c6](https://github.com/oxicrab/oxicrab/commit/66d41c619cf115624be5985c6c6a97ba41b22143))
- Refactoring ([5f882d2](https://github.com/oxicrab/oxicrab/commit/5f882d253f9b4b855f615e577bf50ec97d119a74))
- Updated claude.md ([5c24be4](https://github.com/oxicrab/oxicrab/commit/5c24be492483b566425d16df82fa2d71690589e6))


### Documentation
- Updates to Architecture.md ([04ed7a1](https://github.com/oxicrab/oxicrab/commit/04ed7a18ca7856db6dd84b913bcf54cb952c2464))
- Pre-work: Extracted tests from anthropic_common.rs and cost_guard.rs into ([7bc90e7](https://github.com/oxicrab/oxicrab/commit/7bc90e7fbb3d9789a668dc3c53c50cc36bc0bfb1))
- More docs updates for new config ([e242872](https://github.com/oxicrab/oxicrab/commit/e24287238a53e641d76437f17fe25a29d613f679))


### Fixed
- Fixed some CI perms risks ([2fdbd8e](https://github.com/oxicrab/oxicrab/commit/2fdbd8eb3868a6ca38f963aa1df343e78984267c))
- prevent pipe deadlock in credential helper subprocess ([843e378](https://github.com/oxicrab/oxicrab/commit/843e3784e41b237f03d288522fbf463f87e072bc))
- 58 bug fixes from deep codebase review ([4cdf9e5](https://github.com/oxicrab/oxicrab/commit/4cdf9e59e3a1429308ef8af6f70b5b86ce6a38f7))
- So many bug fixes ([91d6bfa](https://github.com/oxicrab/oxicrab/commit/91d6bfadcec44cd86501605c2ac3f564f726aaca))
- Fixed some bugs ([2c46a5f](https://github.com/oxicrab/oxicrab/commit/2c46a5fdcdf5f6300efd546009e2453aa66c44cc))
- Fixed a few bugs ([e3fe907](https://github.com/oxicrab/oxicrab/commit/e3fe90764282b6d58bef3984098a4c1f1eda891e))


### Security
- 1. Cache middleware fix — after_execute() now receives &dyn Tool and ([f0184cb](https://github.com/oxicrab/oxicrab/commit/f0184cb2660851ea34f586ae8a04746ee932b002))
- Finding: Subagent middleware bypass ([a4d6fe9](https://github.com/oxicrab/oxicrab/commit/a4d6fe997ecfc553bf129fcbed5e27a440c374e6))


### Testing
- Mooore tests ([2d3b2e5](https://github.com/oxicrab/oxicrab/commit/2d3b2e551fedf65d62b2ed8b1466df98b256c3d6))

## [0.10.3] - 2026-02-19

### Added
- Added packages ([418c379](https://github.com/oxicrab/oxicrab/commit/418c3797398d4613830b8730db6d8181d24f6967))
- New Integration Tests ([cd6e63f](https://github.com/oxicrab/oxicrab/commit/cd6e63f77508929ef1356619c9be481f195f48cc))
- Feature 1: Three-Encoding Leak Detection (src/safety/leak_detector.rs) ([d9fba06](https://github.com/oxicrab/oxicrab/commit/d9fba061f2d8badf8f6add18b2cad39c30da1d29))
- New file: ([a5e094c](https://github.com/oxicrab/oxicrab/commit/a5e094c29a952a50ba593f46f0f24a1f80f28d62))
- New docs ([71c7d4c](https://github.com/oxicrab/oxicrab/commit/71c7d4cea47db246f2a25336d9682382399b5bf5))
- src/agent/tools/todoist/mod.rs — 5 new actions added: ([883f709](https://github.com/oxicrab/oxicrab/commit/883f7096ce2b260e8662034f213186d803f2aafa))


### Changed
- reqwest 0.12 → 0.13 Upgrade Summary ([baf65cd](https://github.com/oxicrab/oxicrab/commit/baf65cd46d662dad8e62737e4cc587b077eec646))
- Replace gitleaks with trufflehog ([7a4a6b5](https://github.com/oxicrab/oxicrab/commit/7a4a6b5c4787e585d7bacc47205faab96efff7ee))


### Documentation
- Summary of fixes ([c0db806](https://github.com/oxicrab/oxicrab/commit/c0db806ac5aaa0b125eda3f47feb22cd543b1879))
- Prompt-Guided Tools Fallback ([4c2627d](https://github.com/oxicrab/oxicrab/commit/4c2627dd44368b2becda1c547e0586d248df6613))
- Docs upadates ([b7f9267](https://github.com/oxicrab/oxicrab/commit/b7f9267713e3d3c61e96f1c7da2b0146b03db346))


### Security
- 1. Security patterns -- curl/wget file upload exfiltration ([3d774a4](https://github.com/oxicrab/oxicrab/commit/3d774a4e69f0ea296e5142716dc3b5172be0c218))
- Some security fixes ([6848d9c](https://github.com/oxicrab/oxicrab/commit/6848d9c785d5b797103e9572d5c5c91fe1f6b01c))

## [0.10.2] - 2026-02-18

### Added
- Fixed build issues, dropped macos Intel support ([a4939b7](https://github.com/oxicrab/oxicrab/commit/a4939b7e3defc17149ef954216daef0b61417beb))
- Added credentials helper and keychain support ([273469c](https://github.com/oxicrab/oxicrab/commit/273469c08fb153f2d00b8aa8fe00d496b9ea66f0))


### CI/CD
- bump actions/upload-pages-artifact from 3 to 4 ([e248515](https://github.com/oxicrab/oxicrab/commit/e2485154b0e57122082f0274b453b1740d9c13a2))
- 1. Dependabot — .github/dependabot.yml — weekly checks for both Cargo deps ([58193b8](https://github.com/oxicrab/oxicrab/commit/58193b890760269936cc7702090a6ff036c2cbd5))


### Changed
- use std::sync::Once to ensure the permission ([cf95748](https://github.com/oxicrab/oxicrab/commit/cf95748b38ca093d7d18bd4d30b5cfc88edd1cfa))
- Security Hardening — Phase 1 Summary ([aebc8a2](https://github.com/oxicrab/oxicrab/commit/aebc8a2031069dbe0b024faf757a0a7485b73136))


### Dependencies
- Packages updates for rmcp, dirs, and fastembed ([03ca622](https://github.com/oxicrab/oxicrab/commit/03ca622f79ecf8568afdc3436911b3a2b6cce5bd))
- bump uuid from 1.20.0 to 1.21.0 ([61399ce](https://github.com/oxicrab/oxicrab/commit/61399ce9f3827695189ee77f47e095a5ab986acc))
- bump futures from 0.3.31 to 0.3.32 ([a1d4a86](https://github.com/oxicrab/oxicrab/commit/a1d4a86684ba5cfe72c8e04a61d7d25731ad7bca))
- bump clap from 4.5.57 to 4.5.59 ([ea5121d](https://github.com/oxicrab/oxicrab/commit/ea5121d84d75fb05ba9ab200dabb842c8a87794c))


### Documentation
- Security updates to docs ([fcb5924](https://github.com/oxicrab/oxicrab/commit/fcb5924a1f6092a1b8e45023da1ea077c7d7552c))


### Fixed
- Fixed docs and openssl include ([af9b337](https://github.com/oxicrab/oxicrab/commit/af9b3370395f0dbcd4f0f99f9a66c2ece5a42833))
- Fixed me some bugs ([6f5bafb](https://github.com/oxicrab/oxicrab/commit/6f5bafb09e694e6baadfe8417d814cbf96223a41))
- Fixed some clippy issues ([cafb384](https://github.com/oxicrab/oxicrab/commit/cafb384e5bc393d4763609e64e383777c37edf93))


### Testing
- Did some tests re-organization ([a66175f](https://github.com/oxicrab/oxicrab/commit/a66175f1dbf6305d15b0dcab738b8e8e98cf709b))

## [0.10.1] - 2026-02-17

### Added
- Added costguard, circuit breaker, and doctor command ([ab9c779](https://github.com/oxicrab/oxicrab/commit/ab9c779d583de2fd4d7bcc5ea41f3b9ac1bc1947))
- Part 1: GitHub Tool Expansion (src/agent/tools/github/mod.rs) ([780a560](https://github.com/oxicrab/oxicrab/commit/780a560ad60b36f94d0037b89c12dfc7a658b75f))


### Changed
- Updated GH Actions versions ([0422873](https://github.com/oxicrab/oxicrab/commit/0422873e9b6f933efc6a42313fdeceeba90a3397))


### Documentation
- v0.10.1 - Added logging, docs, upgrade to edition 2024 ([97f9afc](https://github.com/oxicrab/oxicrab/commit/97f9afc342aafd3d41d3aabddc19ff79072a79a6))
- Docs updates for discord and github ([57994de](https://github.com/oxicrab/oxicrab/commit/57994debfb3c96caddc14bb4fbe5bec71fe18e3b))


### Removed
- removed composing reply message ([ea907b5](https://github.com/oxicrab/oxicrab/commit/ea907b5b8e96a03fef2315d0b7195ca1e0fe0735))

## [0.9.6] - 2026-02-16

### Added
- Added release workflows ([5ba6753](https://github.com/oxicrab/oxicrab/commit/5ba6753f1995d1f1245c13c3488651913383a21f))
- Feature 3 - Protected Tool Names: MCP tools can no longer shadow built-in ([c1b6a93](https://github.com/oxicrab/oxicrab/commit/c1b6a932f25ecba0f27f62428275e3e3ef3d328c))
- Two-layer defense against repetitive messages: ([680def6](https://github.com/oxicrab/oxicrab/commit/680def670f768c72cf038f6cecae86c24f326507))
- 1. Discord allow-list (src/channels/discord.rs): Replaced the custom ([e1c72af](https://github.com/oxicrab/oxicrab/commit/e1c72af8d43e5d717f3ed79cf5428bbb52ad10e1))
- Added logo ([fb92f3b](https://github.com/oxicrab/oxicrab/commit/fb92f3b8ce4fc8e54c4a658ed25f7e94f1ab1b4b))
- Added config example ([dc39e6c](https://github.com/oxicrab/oxicrab/commit/dc39e6c9042d1b81c757de38f888a48fc734cc42))
- Added MCP approach ([e2e0099](https://github.com/oxicrab/oxicrab/commit/e2e0099927cde62aa0e9e24f597a3d179cd3ea1b))
- Image gen tool ([b83eabc](https://github.com/oxicrab/oxicrab/commit/b83eabc5ba0d50d2070300b64577da80293c7ad8))
- 1. Browser eval IIFE wrapping — action_eval() now detects return statements ([5523683](https://github.com/oxicrab/oxicrab/commit/5523683097b6ee3ebcb9314316e2fa427e05aa08))
- Added media support ([15af2df](https://github.com/oxicrab/oxicrab/commit/15af2df556ebd6ddb7383d1d0a67d77761faff7d))
- Added more tests ([75faaba](https://github.com/oxicrab/oxicrab/commit/75faabad44b188a34cb285dd2aa7f0bc18dee4aa))
- Added twilio provider ([f24e430](https://github.com/oxicrab/oxicrab/commit/f24e430012082794c87f1c1a770eefc5bc377b32))
- Added openai generci provider ([8af51f9](https://github.com/oxicrab/oxicrab/commit/8af51f9c9a11e848791ec276afffd08c1a6138f2))
- Added CLAUDE.md file ([c2ca167](https://github.com/oxicrab/oxicrab/commit/c2ca167d2a6cda7b22d32dbce1956cf5c0d84972))
- Tools in memory ([dc4bb96](https://github.com/oxicrab/oxicrab/commit/dc4bb96899c8d975cd57d1bcb473c578fe7bc643))
- Obsidian tags ([a3bf49f](https://github.com/oxicrab/oxicrab/commit/a3bf49f1b6c0399570872f34a62be9072637fa21))
- Channel architecture ([8a5ab07](https://github.com/oxicrab/oxicrab/commit/8a5ab074d46120c2d74b8f8f2402fc5024ed0d78))
- Added cron expires / run count ([fd2730a](https://github.com/oxicrab/oxicrab/commit/fd2730a8d0f231332da7d25d4191ecaec9f6f075))
- Obsidian tool ([89ec51c](https://github.com/oxicrab/oxicrab/commit/89ec51cfd54da9a67e95adcd8afc97f5cc270db3))
- Fix image support ([0fc0042](https://github.com/oxicrab/oxicrab/commit/0fc00421e40c1fe19bfef7908fcd285afa18ea37))
- Added image support ([2b100e6](https://github.com/oxicrab/oxicrab/commit/2b100e629cc841cbc3d1635e0a82cf1761d95e84))
- Added token heuristics update ([cabfbea](https://github.com/oxicrab/oxicrab/commit/cabfbeac9aa2daad4e60d75d49ba524129994155))
- Add verbose debug logging to Todoist pagination ([0247d37](https://github.com/oxicrab/oxicrab/commit/0247d37fdfe4fa4a43e6898a0630cb42e38c9419))
- Add debug logging to Todoist pagination for diagnostics ([961fa63](https://github.com/oxicrab/oxicrab/commit/961fa63c5fe38e95e688bb5cfcd4087c028c6fb6))
- Add tests for tmux, todoist, and streaming edit fixes ([fc32c7d](https://github.com/oxicrab/oxicrab/commit/fc32c7dec1b30dcae703cee83862d59335af1eeb))
- Add silent responses, sender ID context, memory search tool, and channel streaming ([3d98704](https://github.com/oxicrab/oxicrab/commit/3d987042de7e3c73efbd5eeadab963440d08e021))
- Added media tool ([0fdfe4c](https://github.com/oxicrab/oxicrab/commit/0fdfe4c68436fc53f1e49c80049a0c57d3e222fe))
- Feature 1: Typing Indicators — src/agent/loop.rs ([3599db3](https://github.com/oxicrab/oxicrab/commit/3599db312a108df079350c3a714a7d7b5386c7a6))
- Parralel tool operations ([8f82af1](https://github.com/oxicrab/oxicrab/commit/8f82af1e9b2086abf0d5c014e9b4f28c6f3b6e16))
- Parralel tool operations ([bd09df9](https://github.com/oxicrab/oxicrab/commit/bd09df97afa935f70175cf30793d4b5c04e353e1))
- Added agents.md backup ([7019e6d](https://github.com/oxicrab/oxicrab/commit/7019e6d309e1ee3d01c84b245fbef6a76890e1ca))
- Add run to cron action ([22424ea](https://github.com/oxicrab/oxicrab/commit/22424ea3b2790a30d2dcb753f9caa1b3562319e1))
- Multi-channel ([773edb4](https://github.com/oxicrab/oxicrab/commit/773edb4b052c544c1295bf710dfd1a2254ec98bc))
- Added typing indicators ([5eddcd9](https://github.com/oxicrab/oxicrab/commit/5eddcd983889394f7569630b46af43682dc58a87))
- Added streaming ([fb4aba7](https://github.com/oxicrab/oxicrab/commit/fb4aba72ef638116c2ac6222a9c65550ff9d4e71))
- Added Github, todoist etc tools ([1e4ecde](https://github.com/oxicrab/oxicrab/commit/1e4ecde008e8f8a64b08ee88aa111ec04999ecd1))
- Base implementation ([a4c3a76](https://github.com/oxicrab/oxicrab/commit/a4c3a76b4431a3e4acc2127eafdf27856e6869c9))


### Changed
- Inspired by IronClaw's design — once the message tool has been used, it's ([1152d8b](https://github.com/oxicrab/oxicrab/commit/1152d8b9e157e90c1a59d1e75f86d6039ac0e3ec))
- Problem: After the message tool sends a response to the user, the reflection prompt 'Review the results and continue. Use more tools if needed, or provide your final response to the user' was being interpreted by the LLM as an invitation to send yet another message — leading to 3 near-identical summaries. ([0214b58](https://github.com/oxicrab/oxicrab/commit/0214b588adc38a087b836419f918e9f35562cea5))
- Split out more test code ([e967355](https://github.com/oxicrab/oxicrab/commit/e96735516a1367e513bba61b85a14b04ab49e345))
- Renamed to oxicrab ([430bac6](https://github.com/oxicrab/oxicrab/commit/430bac6bc3d0b51995dcae5a3719beb37f97a597))
- Reversed cloud and local fallback ([52a5eff](https://github.com/oxicrab/oxicrab/commit/52a5efff73f3e0f1b5f2061e9fc03921dfc8e25b))
- Debug heartbeat ([fbfe21c](https://github.com/oxicrab/oxicrab/commit/fbfe21c38430cef9cdf02edac4a5c5f37b6cff45))
- Better status ([defc938](https://github.com/oxicrab/oxicrab/commit/defc938597499d3995cd8e78608c954d51699fe0))
- Some obsidian updates ([16999ee](https://github.com/oxicrab/oxicrab/commit/16999eeebdfcb0384c13e865dd3252243760ca3f))
- Update README with feature flags, agent improvements, and architecture details ([5388c44](https://github.com/oxicrab/oxicrab/commit/5388c44522a9050ca4eee3ad563cc24ae00c2224))
- Slack emoji ([7e9cac8](https://github.com/oxicrab/oxicrab/commit/7e9cac8acf9b08900e3b41325dbcff3eb373d8f4))
- Rust refactoring ([70fb8a2](https://github.com/oxicrab/oxicrab/commit/70fb8a2be3afaad58ecc50dd0dc4899975ef79f9))
- Split tests ([785df71](https://github.com/oxicrab/oxicrab/commit/785df715abf50b0cd09661db94656ef0fcc48531))
- Use config max_tokens for LLM responses, increase tool result limit ([4c5db6c](https://github.com/oxicrab/oxicrab/commit/4c5db6c0fab9484d07aaab19047d64bb75b13913))
- Construct task URL from ID per v1 migration guide ([da3973a](https://github.com/oxicrab/oxicrab/commit/da3973a7e3af844922a48d672e3e953e8dd7d080))
- Use correct v1 API endpoints for Todoist filter queries ([a466b61](https://github.com/oxicrab/oxicrab/commit/a466b617bf143d67f13ff28c00753ff448b33945))
- Paginate Todoist API responses to fetch all tasks/projects ([b251580](https://github.com/oxicrab/oxicrab/commit/b251580a48d0433cc554b3f702cabd950d94a46d))
- Handle Todoist v1 paginated response format ([a7b2fc6](https://github.com/oxicrab/oxicrab/commit/a7b2fc6be400032c39d7f9d8807efd0712588751))
- Auto-create tmux sessions on send/read instead of returning error ([b2f5015](https://github.com/oxicrab/oxicrab/commit/b2f50156bb67c361717debd073d47aea045b27f3))
- Update Todoist API from deprecated v2 to v1 ([7e246f2](https://github.com/oxicrab/oxicrab/commit/7e246f2fa8caf9a9e1750ca44c26d369e3569961))
- Tool choice ([972141b](https://github.com/oxicrab/oxicrab/commit/972141b8b1601465c65c64d49287a866d6923931))
- Moved to agents.md ([660e9c0](https://github.com/oxicrab/oxicrab/commit/660e9c096068f91bbd5e4c2eda9892aca72ae725))
- Action directive and reduced WhatsApp log spam ([ce9a6e2](https://github.com/oxicrab/oxicrab/commit/ce9a6e2fa9b16613e0f609f89fcf5336842becf2))
- Restart skip ([9d5dcd2](https://github.com/oxicrab/oxicrab/commit/9d5dcd26107f9e8661152831c7539a74dd2b8b22))
- Refactoring ([9c28d3c](https://github.com/oxicrab/oxicrab/commit/9c28d3caeb4c0592777f72feb207d60612853da3))
- Updated CI ([a276123](https://github.com/oxicrab/oxicrab/commit/a2761238c8c8a8d223a92fc4a3b554b2c499393f))
- Refactoring ([c89fe5f](https://github.com/oxicrab/oxicrab/commit/c89fe5f6179433368b854d0d1a2e76ccdbf91dbe))
- Refactoring ([32aab84](https://github.com/oxicrab/oxicrab/commit/32aab841d5ef0d0679cb3853bf246e7c82f21dbd))


### Dependencies
- Crate upgrades ([f9a1b67](https://github.com/oxicrab/oxicrab/commit/f9a1b6717ffcc7c3b43c63d28f7fa096d9987f37))


### Documentation
- Docs and logos ([dc20667](https://github.com/oxicrab/oxicrab/commit/dc2066749b5a12cc543bb062380701845375ac6d))
- Docs, Docker, deployment, more docs ([e7a9ee9](https://github.com/oxicrab/oxicrab/commit/e7a9ee98aa47decf36f96ff50e03b6b0457c673d))
- Summary of the fix: Moved cleanup logic into a Drop impl on BrowserSession. ([dfc9d3c](https://github.com/oxicrab/oxicrab/commit/dfc9d3cd649c89fa1c4e44e028a20e1215e340b4))
- README updates ([52150b5](https://github.com/oxicrab/oxicrab/commit/52150b51c25baf4de677ec3d6c93135c68b21758))
- README updates ([2b54b1f](https://github.com/oxicrab/oxicrab/commit/2b54b1fd85e1dcd7269ff81daffb47c55871bf9d))


### Fixed
- Fixed: src/channels/telegram.rs:157-163 — replaced inline media ([1f8431a](https://github.com/oxicrab/oxicrab/commit/1f8431ab7e5d4f16f5d58c850be070b7272f3ec9))
- Fixed up tools layout ([1a8eaf4](https://github.com/oxicrab/oxicrab/commit/1a8eaf46358abc4fa39b1f1465658fb04066012e))
- Fixed Cargo.toml and added license ([3a1cbdd](https://github.com/oxicrab/oxicrab/commit/3a1cbddd896d23f5abeeb1b3b5976a82ad9f54c8))
- More fixes ([a0b3cf2](https://github.com/oxicrab/oxicrab/commit/a0b3cf2bbc503f42765453dde35214de49ca3ccc))
- More image fixes ([939757b](https://github.com/oxicrab/oxicrab/commit/939757b48e872f62ee249cb3b75d282762667841))
- Fixed image build ([4ea826b](https://github.com/oxicrab/oxicrab/commit/4ea826b39c5fa3335a900dadd4c633621bbf4893))
- Fixed fallback and local model ([8242b6f](https://github.com/oxicrab/oxicrab/commit/8242b6f151d9d82e06ee80f8e8b54edb1ea24b37))
- Fix cron loop ([e5e83e2](https://github.com/oxicrab/oxicrab/commit/e5e83e21388e915ff14b6d9868298d53ae7045ce))
- Clippy and bug fixes ([0a3d8a3](https://github.com/oxicrab/oxicrab/commit/0a3d8a381336840b47cd50a5ed032f8e62ce7701))
- Consistency fixes ([7b1a518](https://github.com/oxicrab/oxicrab/commit/7b1a5185ac46abe84f13d637d37c0dc7e2ffd4cf))
- Fixes ([42f56f3](https://github.com/oxicrab/oxicrab/commit/42f56f34f8e9f2e2a82f966fb53c286eb13881e5))
- Fixes to cron etc ([9392534](https://github.com/oxicrab/oxicrab/commit/93925346e601f78ecd45b4d07040bad83d447c5b))
- Fixes to cron etc ([015036a](https://github.com/oxicrab/oxicrab/commit/015036aad4a9019b2d9aaf520092626abc351db7))
- Subagent improvements (5 features): ([8f46c77](https://github.com/oxicrab/oxicrab/commit/8f46c77dcc85dbf3f8fea2c65ac0f6c200ec836d))
- Fixed whatsapp images ([5326ce0](https://github.com/oxicrab/oxicrab/commit/5326ce0e54d0eb2010d9534d21f8191c4db7584e))
- Various bug fixes ([219d162](https://github.com/oxicrab/oxicrab/commit/219d1626a9641b71b8b85ed98085a64c23e4cf84))
- Fix Todoist filter endpoint and priority order per official docs ([7613dd8](https://github.com/oxicrab/oxicrab/commit/7613dd8190a0edc9255022b018d26695f45d151d))
- Fix Todoist filter to use /tasks with query param, add limit=200 ([bf48f14](https://github.com/oxicrab/oxicrab/commit/bf48f1467dac302653421f61705150e80b988ce7))
- Fix streaming edits overwriting previous bot messages ([729c253](https://github.com/oxicrab/oxicrab/commit/729c253897418f0947c8f43b0609e9aef41722dc))
- Fix Todoist API base URL: /api/v1 not /rest/v1 ([cad1c42](https://github.com/oxicrab/oxicrab/commit/cad1c425249d6e3027871b43b9c6d07cb0ed8a4e))
- Fix tmux session-not-found errors and todoist response decoding ([508337c](https://github.com/oxicrab/oxicrab/commit/508337c3c26d6114a26aaeea47a2f54234b389a1))
- Whatsapp and cron fixes ([7720976](https://github.com/oxicrab/oxicrab/commit/77209767d3e87928862ba98eb4580c6f57315f98))
- TZ fix ([b7d4d2f](https://github.com/oxicrab/oxicrab/commit/b7d4d2f20affb55132a191e83eeb93e7c99dcf84))
- Fixed message send ([c99d6a0](https://github.com/oxicrab/oxicrab/commit/c99d6a02abed3931f8d64eac09723281fd11bcb7))
- Fixed test ([0bcc155](https://github.com/oxicrab/oxicrab/commit/0bcc1554f88c60d3cfd3a6ed21065d25dc5c758e))
- Many cron fixes ([ac75fc9](https://github.com/oxicrab/oxicrab/commit/ac75fc9546d59bda2a12e183eb439187e06c1c11))
- Fixed context mess ([aa59fb0](https://github.com/oxicrab/oxicrab/commit/aa59fb0cc5ca886a43533d8c2158beee48638d5e))
- Fixed tools ([f728b15](https://github.com/oxicrab/oxicrab/commit/f728b157b9d18217633a9f6003ae5087e653f72a))
- Fixed cron loop ([b7587e1](https://github.com/oxicrab/oxicrab/commit/b7587e180f376495960a18102a61a88d341b4e2f))
- Fix itmeouts ([8f66ac7](https://github.com/oxicrab/oxicrab/commit/8f66ac7af3f5ca63ef7814b51bf6977ec8de623e))
- Fix cron service and add tests ([6345012](https://github.com/oxicrab/oxicrab/commit/63450129b191cfe3e4d5f3c5c2b951ea1adbec13))
- Some fixes ([d83aa47](https://github.com/oxicrab/oxicrab/commit/d83aa47383649b4ecfc26f3774297b64e2f836ae))
- Prevent duplicate cron job names ([18021e0](https://github.com/oxicrab/oxicrab/commit/18021e0db39b421c64517d428ef4783d529e301c))
- Cron fixes ([79046d4](https://github.com/oxicrab/oxicrab/commit/79046d49caaa5132aeb2c7d1e629d77f1ec48045))
- Fixes for build ([19e35e3](https://github.com/oxicrab/oxicrab/commit/19e35e369c264c6c79ff8cf696fcdd0200e4943c))
- More bug fixes ([82c09e7](https://github.com/oxicrab/oxicrab/commit/82c09e736d7708448933e62fa5fd978dce47eae3))
- More bug fixes ([e686e52](https://github.com/oxicrab/oxicrab/commit/e686e527bc00c71cf99a2d1c89da6b2b4e0f78b7))
- Fixed Discord ([de4b370](https://github.com/oxicrab/oxicrab/commit/de4b3707a6ad4bcfa9310161a9378d97cb7f6618))
- More fixes ([218d0a6](https://github.com/oxicrab/oxicrab/commit/218d0a63fef47c407628c1d29488b907efe3290d))
- Fixes ([b7f3334](https://github.com/oxicrab/oxicrab/commit/b7f3334e475ff3f93e6afdfda503fe478624f16b))
- Fixes ([bc4cd58](https://github.com/oxicrab/oxicrab/commit/bc4cd584b3b14634b992e194c500ccb18d1dc49b))
- Fixed vulns ([d4e5e58](https://github.com/oxicrab/oxicrab/commit/d4e5e58a5209ae4c8fcd1b04e3e2ccf54282c998))
- Fixed Slack duplicate messages: ([13a259f](https://github.com/oxicrab/oxicrab/commit/13a259f0847ecf5803fe0dd6a9f86cd86c1a8075))


### Removed
- Removed message tool ([5a860b1](https://github.com/oxicrab/oxicrab/commit/5a860b1d3b585929e86f65407739242b0248efc9))
- Removed the format_tool_status function and the block that sent tool ([59ff52b](https://github.com/oxicrab/oxicrab/commit/59ff52b0d0362d9a9e578646585d28ae5d2f9099))
- Removed opensll ([f562d60](https://github.com/oxicrab/oxicrab/commit/f562d60f15e868727a2cfda0c761a7aa794a80c9))
- Removed thinking ([66cac00](https://github.com/oxicrab/oxicrab/commit/66cac00ab2ba9d5c81b3935a3a3cf2c9528e915a))
- Removed streaming ([ef350aa](https://github.com/oxicrab/oxicrab/commit/ef350aaf2f7913def73a049e720d0492396bd893))
- removed trivial tests ([1dd3c45](https://github.com/oxicrab/oxicrab/commit/1dd3c450122d58433f897c75d1ce1c92173a3e81))
- Remove verbose debug logging from Todoist pagination ([ca45f1d](https://github.com/oxicrab/oxicrab/commit/ca45f1db678fcc524f6a0221a53ff42203a8ad02))
- Remove url field from create_task response (not in v1 API) ([3ee1852](https://github.com/oxicrab/oxicrab/commit/3ee1852f8fdb815e37e2c7efa65596a7bdf5e5e9))


### Security
- More anti-hallucination ([e73b8d7](https://github.com/oxicrab/oxicrab/commit/e73b8d7aff9e8c19eedc542293a3a9f1f203bb97))
- Security updates ([614cd2b](https://github.com/oxicrab/oxicrab/commit/614cd2b475f7004628387b5878e896b3f81b17f6))


### Testing
- More integration tests for event-based cron ([8c6d4b0](https://github.com/oxicrab/oxicrab/commit/8c6d4b02a7fd6d6ac261be149ad1247c61b9bf64))
- More tests ([20d68a6](https://github.com/oxicrab/oxicrab/commit/20d68a6d4197ba350f5e4267aee2f4342a62c366))
- Integration tests ([336aa93](https://github.com/oxicrab/oxicrab/commit/336aa938ed239a19ca43d6bc023b0dc51d670348))
- Initial tests ([5c98205](https://github.com/oxicrab/oxicrab/commit/5c982050e7d4e16fb84476ac48c27419f9331d5d))


