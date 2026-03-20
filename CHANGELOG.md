# Changelog

All notable changes to this project will be documented in this file.

## [0.16.2] - 2026-03-20

### Documentation
- fix stale README version and CLAUDE.md toolchain reference ([e139d7b](https://github.com/oxicrab/oxicrab/commit/e139d7be88ce813fd34b28279d3c2c8f8f31802e))


### Fixed
- **docs:** reference actual rust-toolchain.toml (pins stable 1.94.0) ([62f4122](https://github.com/oxicrab/oxicrab/commit/62f4122bf1d84bf7015d52b7b43a4df252fe8772))
- **channels:** automatic reconnection after network outage ([d7e2961](https://github.com/oxicrab/oxicrab/commit/d7e296154f3f16670702207e7b3dfeb260c7b727))


### Testing
- remove theatre, add real gap coverage ([1972ab8](https://github.com/oxicrab/oxicrab/commit/1972ab8fede8e112de1b005b6459fe057b89068b))
- **channels,safety:** add supervisor, reconnection, and safety integration tests ([d96b5b7](https://github.com/oxicrab/oxicrab/commit/d96b5b70b7c2f876ba7098fb4d1a2997092ba683))

## [0.16.1] - 2026-03-20

### Added
- **telegram:** bring channel to parity with Discord and Slack ([319e613](https://github.com/oxicrab/oxicrab/commit/319e6131a8552adce950ca28eee4b0108b079e40))
- **memory:** add management actions, semantic dedup, query expansion, turn-based compaction ([7b734dc](https://github.com/oxicrab/oxicrab/commit/7b734dca50492a4b063c1d494e756413b533ac86))
- **memory:** token-budget-aware memory context injection ([2bb03eb](https://github.com/oxicrab/oxicrab/commit/2bb03ebeb7a565848e7b02128132574dcac7e3b0))
- **memory:** make retention days configurable ([a220e55](https://github.com/oxicrab/oxicrab/commit/a220e55d08b80833386576bc3cec727c2b3d6bc8))
- **memory:** make search result limit configurable ([6265ee6](https://github.com/oxicrab/oxicrab/commit/6265ee6a91cb0d83136641021e7bb9f2795aa68c))


### Changed
- Update docs examples to TOML ([9c7b366](https://github.com/oxicrab/oxicrab/commit/9c7b366e787f816cb3c7af68b475fd6a26d26027))


### Documentation
- update channel, config, and tool documentation ([c50de7f](https://github.com/oxicrab/oxicrab/commit/c50de7ff4ca42623b68b418f39d0a9afc7a8b7ff))
- Refresh architecture and public docs ([41f99d7](https://github.com/oxicrab/oxicrab/commit/41f99d7efd321c31ff9967cdc7f09dbf1e01b3a8))


### Fixed
- **cli,bus,router,observability:** comprehensive polish across remaining subsystems ([d45f146](https://github.com/oxicrab/oxicrab/commit/d45f146867f417899acaf6f973771eb56e7d2cf9))
- **security,config,mcp,tests:** harden security patterns, config validation, MCP lifecycle ([935c777](https://github.com/oxicrab/oxicrab/commit/935c77726f477ec7c1dab83a39788d29c3f97db6))
- **agent-loop:** shutdown signal, lock safety, hallucination accuracy ([49c16e6](https://github.com/oxicrab/oxicrab/commit/49c16e6508afc63890192c178ecde394bb52b65a))
- **ci:** revert gateway to non-optional dep, fix gmail depth test ([561e323](https://github.com/oxicrab/oxicrab/commit/561e323e98e6fdd989e5ab8e18b4f2005a2b20bc))
- **channels,providers:** comprehensive channel and provider hardening ([04d1b27](https://github.com/oxicrab/oxicrab/commit/04d1b27f4f9b55dca10a4eab17a0edbb89e20ea8))
- clean up stale comments, dead code, and missing Tasks scope ([c997e3a](https://github.com/oxicrab/oxicrab/commit/c997e3a5c448eca887a6022287a08764a083e57e))
- **tools:** security hardening, pagination, per-action approval across 6 tools ([5b9214d](https://github.com/oxicrab/oxicrab/commit/5b9214d93b5d869810caad0efc286b5dada51472))
- **memory:** validate embedding dimensions and warn on model change mismatch ([95616fb](https://github.com/oxicrab/oxicrab/commit/95616fbed9b758b612dc6aa024afcf418d10323f))
- **memory:** correct hybridWeight semantics and improve FTS ranking ([2fcbfa6](https://github.com/oxicrab/oxicrab/commit/2fcbfa6db7268c66f265e49103daeda8dc2769b1))
- **gateway:** accept case-insensitive hex in webhook signatures ([495ccb8](https://github.com/oxicrab/oxicrab/commit/495ccb88cc059595a79b2f497340f598bb7eb198))


### Maintenance
- ignore cargo-fuzz local artifacts ([4083d87](https://github.com/oxicrab/oxicrab/commit/4083d87960f66c56fdc12e77aa25f972b1453a80))


### Other
- Make homepage hero channel-first ([12997e7](https://github.com/oxicrab/oxicrab/commit/12997e77c343d773ffe83ceb47aa00ca797ec5a7))
- Refresh homepage hero and routing copy ([02fa943](https://github.com/oxicrab/oxicrab/commit/02fa943af18deb2f0429f6398935e74ca29f0734))


### Performance
- **db:** run PRAGMA optimize during hygiene ([36b9ee1](https://github.com/oxicrab/oxicrab/commit/36b9ee1791e43c32a4825abb2d0e055d92570a80))


### Removed
- removed old docs ([4c905da](https://github.com/oxicrab/oxicrab/commit/4c905daccd6dcea0dc0b95d7f555175e024ab2e5))

## [0.16.0] - 2026-03-19

### Added
- Add routing and gateway warning regressions ([fc6fc3c](https://github.com/oxicrab/oxicrab/commit/fc6fc3cf85c882f447fa9d3ff9e149b1dab4dc39))
- Add remember metrics and router trace logging ([532d82d](https://github.com/oxicrab/oxicrab/commit/532d82da1991e47206c5812305ffcf4d263317b7))
- Add hallucination observability counters ([6157069](https://github.com/oxicrab/oxicrab/commit/6157069ab5ed623fb76dd636aea1d2144f0adc98))


### Changed
- Replace deprecated sccache action ([64580be](https://github.com/oxicrab/oxicrab/commit/64580be8ccfb201c1e6355a4eb3d12de21a8265c))
- Migrate config system to layered TOML ([0bba087](https://github.com/oxicrab/oxicrab/commit/0bba0876e076960eda999500da51a5e4a18d0f28))
- extract oxicrab-transcription crate ([2f39dfc](https://github.com/oxicrab/oxicrab/commit/2f39dfcd897fce3fa2e5a6c8ea35b04128383856))
- Migrate WhatsApp channel to wa-rs and move toolchain to stable ([3524fd9](https://github.com/oxicrab/oxicrab/commit/3524fd94b46833e6f1ecadff4d55b55175321756))
- **auth:** merge Google OAuth into oxicrab-tools-google crate ([9122570](https://github.com/oxicrab/oxicrab/commit/912257058124da21ba9a778c3d5493fe6beddfe8))
- **session:** merge session manager into oxicrab-memory crate ([c2402d4](https://github.com/oxicrab/oxicrab/commit/c2402d432b1afc7d56f403595bae38fddb5b1b70))
- **tools:** extract oxicrab-tools-rss crate ([84bad1f](https://github.com/oxicrab/oxicrab/commit/84bad1f570d283df1c8d5fa85f98be855a938d2a))
- **safety:** extract oxicrab-safety crate ([0c3c7cb](https://github.com/oxicrab/oxicrab/commit/0c3c7cba651ac3b044c17c505460ab2ee600c54c))
- **router:** extract oxicrab-router crate ([31896c6](https://github.com/oxicrab/oxicrab/commit/31896c6223ce5b0a34fadb696eda04531e25c689))
- remove extracted tool source files from binary crate ([5891d8c](https://github.com/oxicrab/oxicrab/commit/5891d8c13180db30d362de0832d20a3ea5d7d2b2))
- **core:** move shared http/media/url_security utils to oxicrab-core ([45006cd](https://github.com/oxicrab/oxicrab/commit/45006cd56f13b86ca8336c81fbb0676811a682c1))
- **tools:** extract obsidian tool into oxicrab-tools-obsidian crate ([1f70191](https://github.com/oxicrab/oxicrab/commit/1f70191e8cb7684a6ed7f1de2990c91f069a16e5))
- **tools:** extract browser tool into oxicrab-tools-browser crate ([7839d22](https://github.com/oxicrab/oxicrab/commit/7839d227927aca901758aacf8da046a312188d66))
- **tools:** extract Google tools into oxicrab-tools-google crate ([10e4f30](https://github.com/oxicrab/oxicrab/commit/10e4f30814ca62b5a65a2d05656fd8b6fc9d4934))
- **tools:** extract API tools into oxicrab-tools-api crate ([b14fd83](https://github.com/oxicrab/oxicrab/commit/b14fd837448e4190ded70a2200d65b8f5d43143a))
- **tools:** extract system tools into oxicrab-tools-system crate ([0a25c89](https://github.com/oxicrab/oxicrab/commit/0a25c89fcb8696f4eec7d21db490d9c5caf5521e))
- **tools:** extract web tools into oxicrab-tools-web crate ([b394133](https://github.com/oxicrab/oxicrab/commit/b3941333780612c1afe813c25ac0e17edd8a0ba5))
- **gateway:** extract oxicrab-gateway crate ([865ffca](https://github.com/oxicrab/oxicrab/commit/865ffca92fe58f8e3b0779c9366cae7985e50ef7))
- **channels:** extract oxicrab-channels crate ([6d662a7](https://github.com/oxicrab/oxicrab/commit/6d662a7c40a7ad04628dc56b08b69620ee07152c))
- **providers:** extract oxicrab-providers crate ([491b073](https://github.com/oxicrab/oxicrab/commit/491b0731696ba321dccc38096a2712ea5fb9c14d))
- **memory:** extract oxicrab-memory crate ([dc84bcc](https://github.com/oxicrab/oxicrab/commit/dc84bcc7eb6d953283e3500898408d535822a6c9))
- **core:** extract config schema structs to oxicrab-core ([eaf50af](https://github.com/oxicrab/oxicrab/commit/eaf50af6228ea294df04299fb656c697909b8e0d))
- **core:** extract BaseChannel trait to oxicrab-core ([379a37b](https://github.com/oxicrab/oxicrab/commit/379a37b397002d6a63c43199d24eb07d14b21da2))
- **core:** extract Tool trait and types to oxicrab-core ([7bb3f59](https://github.com/oxicrab/oxicrab/commit/7bb3f59ef94be15b15c9f430a6f9f0d84712cef1))
- **core:** extract LLM provider types and trait to oxicrab-core ([3a56dca](https://github.com/oxicrab/oxicrab/commit/3a56dca2f11ea6f2202efeebf72c38bf6f381066))
- **core:** extract bus events to oxicrab-core crate ([56653db](https://github.com/oxicrab/oxicrab/commit/56653db8e9e63d4e107ec7e4f2f944aeecd4e6ce))
- **core:** extract dispatch types to oxicrab-core crate ([c2af3f4](https://github.com/oxicrab/oxicrab/commit/c2af3f4e76fa0950e50541bd40546581900d3081))
- **core:** extract now_ms to oxicrab-core crate ([466d1fe](https://github.com/oxicrab/oxicrab/commit/466d1fe0aa536d2b9f3d97a87e93c007323ab2a4))
- **core:** extract OxicrabError to oxicrab-core crate ([6f29da7](https://github.com/oxicrab/oxicrab/commit/6f29da7b1e11b7809f2b56b725ae837c7e065a1f))
- move provider factory out of config schema ([c4cd740](https://github.com/oxicrab/oxicrab/commit/c4cd740f0f367586a05c3e87bd7038e9772a23fc))
- extract OAuthTokenStore trait to decouple providers from MemoryDB ([b9c4ecc](https://github.com/oxicrab/oxicrab/commit/b9c4ecc0712a896741d4b7b74f5a942d74908a26))
- move StaticRule and DirectiveTrigger to tool base module ([d1968d0](https://github.com/oxicrab/oxicrab/commit/d1968d01e74272534f787899eec4e3c900ee7508))
- Update docs/build.py ([0467b92](https://github.com/oxicrab/oxicrab/commit/0467b92ab319534cd554d133fbf911e4d0be9945))


### Documentation
- update all documentation for workspace crate structure ([13eb96d](https://github.com/oxicrab/oxicrab/commit/13eb96d33bfbee30f6434e202f38ae565e4c08a0))
- add tool crates extraction design spec ([e7e6d3e](https://github.com/oxicrab/oxicrab/commit/e7e6d3e4dd91573338f20d7e548942b810bc8a78))
- add workspace split design spec ([b37b218](https://github.com/oxicrab/oxicrab/commit/b37b2187046184491baeb6d3f6f28fdef6152526))
- document schema migration system in ARCHITECTURE.md ([6341097](https://github.com/oxicrab/oxicrab/commit/63410974adfefcbd1281bb9c2b9172bd55bf6bf6))
- add active-state handling for footer CLI/workspace links ([bbbf2b0](https://github.com/oxicrab/oxicrab/commit/bbbf2b041e7f5e2e013fa346c37f5d3234642a73))
- **routing:** fix flow diagram arrows across all routing layers ([8a6d312](https://github.com/oxicrab/oxicrab/commit/8a6d312e0f425e825c5976b436bb3f4e83747a90))
- **routing:** rename 'Config Knobs' section to 'Router Configuration' ([e249576](https://github.com/oxicrab/oxicrab/commit/e249576183c766b9239a87aed4417c90d5c11634))
- **routing:** replace job-id-dependent rule example with stable commands ([45b7800](https://github.com/oxicrab/oxicrab/commit/45b780074b4fc044c7685ef5000673ed3c11013e))
- **routing:** fix cron rule example to valid run/list patterns ([a230de6](https://github.com/oxicrab/oxicrab/commit/a230de61c6cc9ed84ff8efca048d6759f7ad1c26))
- add contextual CLI and Workspace cross-links ([07d10a5](https://github.com/oxicrab/oxicrab/commit/07d10a5652ce1cf5865495ff8528a79cfb1907f4))
- add routing layers guide and simplify top navigation ([44a4605](https://github.com/oxicrab/oxicrab/commit/44a46059c3257b4e9169931a7200cb9376489db3))


### Fixed
- address final audit findings — RSVP validation, docs, defense-in-depth ([4170611](https://github.com/oxicrab/oxicrab/commit/41706110bb936e8038ab99c91bdfba4fe9a738f6))
- **security:** add prompt guard to display_text and handle it in direct dispatch ([bea0b57](https://github.com/oxicrab/oxicrab/commit/bea0b57d31740cb988173a11d599a339d1466e71))
- **whatsapp:** add file size pre-check before download where possible ([5aa4c64](https://github.com/oxicrab/oxicrab/commit/5aa4c644b2d630ade537029ce248d5666b5f9c84))
- **google:** document in-flight token refresh persistence gap ([01277dd](https://github.com/oxicrab/oxicrab/commit/01277dd5e12e3cdff2e784bf5c424e41528391f4))
- **discord:** attach buttons and embeds in send_and_get_id ([e212698](https://github.com/oxicrab/oxicrab/commit/e2126988d942ac376763c309df81c48bed21a7da))
- **channels:** skip retry for non-retryable channel errors ([cab55b2](https://github.com/oxicrab/oxicrab/commit/cab55b2bc25014c3464679591339e75e11c778de))
- **slack:** document retry_after_secs override chain in classify_slack_error ([c7ec3c3](https://github.com/oxicrab/oxicrab/commit/c7ec3c3a44ddb6e0b990224a38b631aa5375ec22))
- **cron:** enforce minimum 1-second interval and clamp negative cooldown ([f1c87c5](https://github.com/oxicrab/oxicrab/commit/f1c87c50001d19293a6a306d42f5d494bd3e2d77))
- **config:** tighten o1/o3/o4 model inference to require hyphen ([8230bd1](https://github.com/oxicrab/oxicrab/commit/8230bd1f7d977babc7b3fbd19f2f036afba357ef))
- **browser:** correct wait action error message param name ([1bb05de](https://github.com/oxicrab/oxicrab/commit/1bb05dee4d547ecf07f31de58062f5f216e07da9))
- Fix workflow shellcheck issues ([5650bf5](https://github.com/oxicrab/oxicrab/commit/5650bf5b4330f43ca6b08193fe3927d7cb4d5c39))
- Fix CI fuzz target and observability warnings ([fdfc3ea](https://github.com/oxicrab/oxicrab/commit/fdfc3ead5782a95986a10d4daabcd3f6316d50bc))
- Fix nextest archive and fuzz CI on stable ([a1f2b17](https://github.com/oxicrab/oxicrab/commit/a1f2b174fd4e3c4527ccf525e297393078a8be41))
- update release script for workspace crate versioning ([b9ce414](https://github.com/oxicrab/oxicrab/commit/b9ce414c6b3e81cf812b0d098e1905dfc6593afd))
- **test:** provide RememberChecker to router in integration test ([0833e63](https://github.com/oxicrab/oxicrab/commit/0833e638cd33954d594b32a275c856288c26eef1))
- gate channel regex_utils imports behind feature flags ([e86208a](https://github.com/oxicrab/oxicrab/commit/e86208a4d9cada304b87e78a1185f7a18916d185))


### Maintenance
- initialize cargo workspace ([50bb4f0](https://github.com/oxicrab/oxicrab/commit/50bb4f05c6dc08538f771da0a42f5509ccce4c24))


### Other
- Prune stale workspace dependencies ([562c16b](https://github.com/oxicrab/oxicrab/commit/562c16b5ebd3371f0bad6849dd22e44426ffbe90))
- Keep memory search metrics warning-clean ([6f494ac](https://github.com/oxicrab/oxicrab/commit/6f494ac6e1f913dbb63794f2d094352c43a639d1))
- Restore known-good sccache action ([9b5343b](https://github.com/oxicrab/oxicrab/commit/9b5343bd64e949a5b77d27f2f6141475f034706a))
- Align sccache env with working action ([156c4dd](https://github.com/oxicrab/oxicrab/commit/156c4dd7dcb1b4c497c309bfd4a3292c7d708914))
- Export GitHub cache runtime for sccache ([16b8051](https://github.com/oxicrab/oxicrab/commit/16b8051cfd482c35d4b2e08b38fbfba1f98c6450))
- Pin actionlint workflow action ([26250ce](https://github.com/oxicrab/oxicrab/commit/26250cea36584f0187cf7bf37a004abb73ce87d8))
- Harden gateway config and unify workspace dependencies ([370235e](https://github.com/oxicrab/oxicrab/commit/370235edc3c4dd7277b39eb0816db912c9298960))
- Harden routing state and config loading ([86428e3](https://github.com/oxicrab/oxicrab/commit/86428e32251026ad9d21ab6c0fc2ec2bcc390151))
- Isolate agent run state by request and session ([4e315e8](https://github.com/oxicrab/oxicrab/commit/4e315e8d0aa3c04181bae5410792ded1225945aa))
- Optimize CI and release workflow caching ([2ea93f9](https://github.com/oxicrab/oxicrab/commit/2ea93f9e0adeba10399c61ddbc0623611bc40b7c))
- Stabilize doctor pairing store checks ([5c2090c](https://github.com/oxicrab/oxicrab/commit/5c2090c7e86d40cceb26c2d016c498dfccbec5aa))
- Harden leak redaction path and gate critical fuzzing ([f1c3fae](https://github.com/oxicrab/oxicrab/commit/f1c3faef23f635d8e227c495528369437393064a))
- Enable memory embeddings by default ([2ac5a4e](https://github.com/oxicrab/oxicrab/commit/2ac5a4e6ccc2deb010c2c8dbee31c573823a7e21))
- Pin Rust toolchain to stable 1.94.0 ([5842ecd](https://github.com/oxicrab/oxicrab/commit/5842ecd514299910c105b12c00cb0be438790b6d))


### Removed
- Remove leaked root package features ([50b22ba](https://github.com/oxicrab/oxicrab/commit/50b22ba2116d074b29a7d4988720def3cefa3eb1))
- Remove stale browser feature from Docker release build ([715edfd](https://github.com/oxicrab/oxicrab/commit/715edfd1999399652da602e1f92b01abcf92cf8c))
- Remove stray chromiumoxide root dependency ([36d390c](https://github.com/oxicrab/oxicrab/commit/36d390c0db4aea2a7a0c08b9e7f15e83da383d64))

## [0.15.0] - 2026-03-17

### Added
- **observability:** populate build info metadata with compile-time fallbacks ([6e2539a](https://github.com/oxicrab/oxicrab/commit/6e2539a22ca040f31754752ea947f68ffa825550))
- **observability:** add baseline runtime and process metrics ([086e528](https://github.com/oxicrab/oxicrab/commit/086e5281a23cca6f76887fc5567e0d40ad4b9b6d))
- **router:** close remaining gaps with semantic caching, router semantic decision, metrics export, and fuzzing ([bd8a4e0](https://github.com/oxicrab/oxicrab/commit/bd8a4e00afc288d73c76d9a0d1509dd3c69a1d3b))
- **router:** add semantic confidence-margin fallback and quality metrics ([41ee300](https://github.com/oxicrab/oxicrab/commit/41ee3002f1c0a0e094aff9d3910e4b0422141740))
- **router:** add replay telemetry and adversarial routing contracts ([c6e641d](https://github.com/oxicrab/oxicrab/commit/c6e641d45bb244da709bf53bf86861210c58698f))
- **router:** add semantic index subsystem and policy contracts ([bf1ca6e](https://github.com/oxicrab/oxicrab/commit/bf1ca6ef82899074002a8043444cb15e22e167d7))
- **agent:** enforce additionalProperties=false in tool arg validation ([ee4e98c](https://github.com/oxicrab/oxicrab/commit/ee4e98c054878d3a330792634760a07ef03e8ecf))
- **router:** enforce routed tool policies and add semantic tool filtering ([e4e7eba](https://github.com/oxicrab/oxicrab/commit/e4e7ebabd517be5bf2a4bfee30404a674b95e9b5))
- **router:** migrate Cron buttons to structured context ([520440f](https://github.com/oxicrab/oxicrab/commit/520440ff2f69c9de302b397912aa164e98ca4bfb))
- **router:** migrate GitHub buttons to structured context ([5743be5](https://github.com/oxicrab/oxicrab/commit/5743be5e93fa09c67d37a5c1ae039ca215dc1899))
- **router:** migrate Todoist buttons to structured context ([5f71e84](https://github.com/oxicrab/oxicrab/commit/5f71e84d18efda13dd87858438744f138bb818f9))
- **router:** migrate Google Tasks buttons to structured context ([945d08c](https://github.com/oxicrab/oxicrab/commit/945d08c1548c8c39f67111ad7bf9dcb6724a1321))
- **router:** migrate Google Mail buttons to structured context ([44075fd](https://github.com/oxicrab/oxicrab/commit/44075fd21ec717dc95782ec0ed9ddb1c1567c7f9))
- **router:** migrate Google Calendar buttons to structured context ([021690a](https://github.com/oxicrab/oxicrab/commit/021690a244a6ce680625e9144477400e168e0095))
- **router:** RSS tool with structured buttons, directives, and routing rules ([373921f](https://github.com/oxicrab/oxicrab/commit/373921f8879b6758149cf6fcd30f4835e9b3505c))
- **dispatch:** add webhook dispatch config and handler ([c777921](https://github.com/oxicrab/oxicrab/commit/c77792126e406e61242d8b14c610d0675e6de58f))
- **dispatch:** Discord button handler with DispatchContextStore ([49f46a5](https://github.com/oxicrab/oxicrab/commit/49f46a5fbb8d1a82408227d7fe9f91dcbdc64963))
- **dispatch:** Slack button handler creates ActionDispatch from context ([3fb6864](https://github.com/oxicrab/oxicrab/commit/3fb6864fe320232720b46ed489a1220ed82cfb5f))
- **router:** integrate message router into agent loop ([0daa76a](https://github.com/oxicrab/oxicrab/commit/0daa76aa39c7d5c4810d64a81466671441fe89d9))
- **registry:** collect tool routing_rules at registration ([ed7da84](https://github.com/oxicrab/oxicrab/commit/ed7da842072ce6ec82f43c509bfe67516c8589c7))
- **dispatch:** add action field to InboundMessage and AgentRunOverrides ([10d68d7](https://github.com/oxicrab/oxicrab/commit/10d68d7b47ed7ef1c85f93a305a8ac295b1e9cdc))
- **tools:** add routing_rules() and usage_examples() to Tool trait ([3727418](https://github.com/oxicrab/oxicrab/commit/372741882385fad13b46250cacb8272563abb268))
- **config:** add router configuration schema ([84a48cc](https://github.com/oxicrab/oxicrab/commit/84a48cc541dbb8672062b32b98a601fb293fe32b))
- **router:** implement MessageRouter with priority-based routing ([ca1aa6b](https://github.com/oxicrab/oxicrab/commit/ca1aa6b933c8700286a897e6432ef25555b85db1))
- **router:** add static and config rule types ([2d9c646](https://github.com/oxicrab/oxicrab/commit/2d9c64623b51f961c9617cfc2475503510839085))
- **router:** add RouterContext with directive matching ([0d8a029](https://github.com/oxicrab/oxicrab/commit/0d8a029cc440db391b4b84a66c23ccfb6d64a474))
- **dispatch:** add shared dispatch types ([850f5c2](https://github.com/oxicrab/oxicrab/commit/850f5c20931fd8825592a99adba2832ef47a584b))
- **rss:** add 'next' action for one-at-a-time article review ([66fc6a6](https://github.com/oxicrab/oxicrab/commit/66fc6a68c058c9fa2ad74d3c483fb8dfb81a4b15))
- **rss:** implement feed_stats, cron integration, and model wiring ([e6c2fcc](https://github.com/oxicrab/oxicrab/commit/e6c2fcc92308d7a1d252bdeba561deccf0354c51))
- **rss:** implement scanner with concurrent fetch and LinTS ranking ([a356809](https://github.com/oxicrab/oxicrab/commit/a3568092babc1464fbe05a431858b4277f8420da))
- **rss:** implement LinTS model with Bayesian updates ([e61fcc0](https://github.com/oxicrab/oxicrab/commit/e61fcc0158075905b5dd8a279e8cbd4eb0ebe14c))
- **rss:** implement article actions with suggested buttons ([66dc2a6](https://github.com/oxicrab/oxicrab/commit/66dc2a631f3795e825b0a6a0704e6fdc7be6a71d))
- **rss:** implement feed management (add, remove, list) ([e775b4b](https://github.com/oxicrab/oxicrab/commit/e775b4b54f78c53a37ffe9c13200eb9eed0222f5))
- **rss:** implement onboarding state machine with action gating ([8a5ccfc](https://github.com/oxicrab/oxicrab/commit/8a5ccfc2cf2d2408418b9427694b51dfb1b5fd99))
- **rss:** add tool shell with registration and 11-action dispatch ([8e0e64c](https://github.com/oxicrab/oxicrab/commit/8e0e64ce110d00d6785e15d0375b1dae2be992d7))
- **rss:** add DB access layer for RSS tables ([85edc7b](https://github.com/oxicrab/oxicrab/commit/85edc7b39862c7768098ff19f55b4dcc465993b9))
- **rss:** add migration v5 with RSS tables ([7076fcb](https://github.com/oxicrab/oxicrab/commit/7076fcb2b97dbfb3db6ad3bec498e74720bbe392))
- **rss:** add RssConfig and wire through config pipeline ([bd1b7d0](https://github.com/oxicrab/oxicrab/commit/bd1b7d0207fd5e145de020a2c0a4d9d37a9b22dc))
- **cron:** add auto-suggested buttons for job management ([878ef16](https://github.com/oxicrab/oxicrab/commit/878ef1650b26e2e6d55478b6c6f9ab5f2072ea2c))
- **github:** add auto-suggested buttons for issues and PRs ([2125eb9](https://github.com/oxicrab/oxicrab/commit/2125eb966925f609589c7bf5d244eea30cb40279))
- **google_tasks:** add auto-suggested buttons for task actions ([729954d](https://github.com/oxicrab/oxicrab/commit/729954d688043a8e27bc523f027f7cd481a53636))
- **todoist:** add auto-suggested buttons for task actions ([236e887](https://github.com/oxicrab/oxicrab/commit/236e887b1530a394ea054db8f8ac30141568cdc1))
- **loop:** auto-merge tool-suggested buttons into response metadata ([f4ce866](https://github.com/oxicrab/oxicrab/commit/f4ce8661feb3408cec9a0931ad61824f0e77401b))
- **buttons:** add context field for button click round-trip data ([d017ac3](https://github.com/oxicrab/oxicrab/commit/d017ac3e78a2c01854e71e422376e7eb3d57665f))
- **gateway:** add status page with JSON API and HTML dashboard (#92) ([31ba120](https://github.com/oxicrab/oxicrab/commit/31ba1200c05b46ba32a14ba24867dc98efd1fbc8))


### CI/CD
- bump trufflesecurity/trufflehog from 3.93.7 to 3.93.8 ([f141da3](https://github.com/oxicrab/oxicrab/commit/f141da371ac74d6f32ddf78e108eaabb253c5a19))
- bump dorny/paths-filter from 3 to 4 ([9eb1960](https://github.com/oxicrab/oxicrab/commit/9eb1960279e08c47ff2d40c6c97c7ddd05938856))


### Changed
- **router:** remove global metrics state and refresh semantic index dynamically ([b358d08](https://github.com/oxicrab/oxicrab/commit/b358d08022ab05c9b54f139a1ecda48ccb989d63))
- **core:** unify tool execution contracts and adopt crate-backed infra ([952eabf](https://github.com/oxicrab/oxicrab/commit/952eabf1210c434099799c6bf8b05b5f202c780d))
- **router:** isolate model gateway and add replay workflow ([dc0eb8e](https://github.com/oxicrab/oxicrab/commit/dc0eb8e52564749ed56079e3de72ea1f1597cc7d))
- **router:** state-machine context and strict routing policy plumbing ([66320c0](https://github.com/oxicrab/oxicrab/commit/66320c0551bfac0a7c9017a8286c5f2177f91754))
- **router:** add explicit routing policy/state contracts ([e9cc1c4](https://github.com/oxicrab/oxicrab/commit/e9cc1c4f0aa783a941f0b78ad5b7e187689b701d))
- Revert "Removed specs" ([e2f97c6](https://github.com/oxicrab/oxicrab/commit/e2f97c6fa5a513fef7167ca35664cb6a9b855ac3))
- **core:** split channel/gateway helpers and streamline message processing flow ([fbb8fa8](https://github.com/oxicrab/oxicrab/commit/fbb8fa8a27447995742ae4a207c16f19d02b0045))
- apply cargo fmt to recent changes ([ea4134b](https://github.com/oxicrab/oxicrab/commit/ea4134b43b203cd7eef954e719ca31c076267f73))
- format integration test file ([ba06636](https://github.com/oxicrab/oxicrab/commit/ba06636ccf4358c34c18090c346a3b240480dc5e))
- apply cargo fmt to branch ([617c773](https://github.com/oxicrab/oxicrab/commit/617c7735e7e6d40d859f7a5c0a8dee2feaa4fbaf))
- extract display/button metadata functions into metadata.rs ([6baca92](https://github.com/oxicrab/oxicrab/commit/6baca923e1e4ae4f302637f909a4dd41016d7990))
- extract build_pinned_client() to eliminate 5 copies of SSRF-safe client construction ([07f6f1a](https://github.com/oxicrab/oxicrab/commit/07f6f1ae8d36f8f21a767bbd1c965ad7b3a973e8))
- remove redundant build_execution_context() wrapper ([a873076](https://github.com/oxicrab/oxicrab/commit/a8730768846b1c4efafa12720a221ceccdbfeeb6))
- extract get_compacted_history_timed() to eliminate 3 copies ([8fb8e0d](https://github.com/oxicrab/oxicrab/commit/8fb8e0d3b2e6c54cc31b74cfbd5689d195a870f1))
- **router:** add DispatchSource::label() to eliminate manual match ([ad10ef3](https://github.com/oxicrab/oxicrab/commit/ad10ef3546defb1c259f631a8bea1b01b0bb8af2))
- extract check_prompt_guard() to eliminate 4 copies ([0d68ccd](https://github.com/oxicrab/oxicrab/commit/0d68ccd0b1a6c3f57a244985bb847e78666b950a))
- **db:** extract lock_conn() helper to eliminate ~120 lines of boilerplate ([c60d2c7](https://github.com/oxicrab/oxicrab/commit/c60d2c768f30a76616e35f0065a1f0c04376f7e9))
- **channels:** consolidate shared download size constants ([c7f3ffa](https://github.com/oxicrab/oxicrab/commit/c7f3ffa4b83a05b7e123c62fae56b677d022b705))
- **dispatch:** replace manual LRU with lru::LruCache ([3f03374](https://github.com/oxicrab/oxicrab/commit/3f03374319aae25f0458fcb341083fa887891c80))
- extract redact_dispatch_params() to eliminate duplication ([7827165](https://github.com/oxicrab/oxicrab/commit/7827165e59961df181d843a8313d55a9d4ab1b6c))
- **test:** remove duplicated synchronous tests from integration file ([9c248a3](https://github.com/oxicrab/oxicrab/commit/9c248a3facb34a8cdb5cd0c9672e62a41f9a4123))
- consolidate now_ms() into single shared utility ([9114a2b](https://github.com/oxicrab/oxicrab/commit/9114a2bbdb490e27e1136873a54e9c59c0ee0da4))
- extract AgentLoopResult::to_assistant_extra() to eliminate 3 copies ([010452a](https://github.com/oxicrab/oxicrab/commit/010452a5531bfe9743d77059dec406246b0815ac))
- **db:** extract invalidate_embedding_cache() helper ([ec4a99a](https://github.com/oxicrab/oxicrab/commit/ec4a99a82c0f8a63dff1f44a69e73b385b0e011e))
- extract with_buttons() into ToolResult to eliminate 6 copies ([a3c763a](https://github.com/oxicrab/oxicrab/commit/a3c763a6a7d66dcc33030217883449fd45ecf4ea))
- **db:** move get_recent_daily_entries query into MemoryDB ([376c050](https://github.com/oxicrab/oxicrab/commit/376c0509c9ab268ac0501adbb83f1d2afde20a5f))
- **db:** standardize on conn.transaction() for all transactional operations ([6cebd39](https://github.com/oxicrab/oxicrab/commit/6cebd3979e752fe5d4528db1d7f7bde856ca742b))
- minor code quality fixes from second review pass ([3389aee](https://github.com/oxicrab/oxicrab/commit/3389aeeceacb3f77ccfb229c44e592d6805178b2))
- code quality improvements from review suggestions ([2269576](https://github.com/oxicrab/oxicrab/commit/2269576dd746e102e8fdb81609be9117557e4f8f))
- gut hallucination detection to Layer 1 only ([e039c15](https://github.com/oxicrab/oxicrab/commit/e039c153aa9695ae32b84ccf2a5b4954c5e6a66f))
- remove intent classification and tool filter modules ([9afe76f](https://github.com/oxicrab/oxicrab/commit/9afe76f2c91222b1041b622ad5f35d27b93d59a5))
- **rss:** consolidate now_ms, make browse ranking read-only ([c9fcfec](https://github.com/oxicrab/oxicrab/commit/c9fcfec30b50e6c47c59133c1e998c85df45fa65))
- **loop:** propagate ToolResult through execution pipeline ([11f922e](https://github.com/oxicrab/oxicrab/commit/11f922ebf6aef1a61b13eae2ecaeb5fa23346a7e))


### Dependencies
- bump clap from 4.5.60 to 4.6.0 ([35fe330](https://github.com/oxicrab/oxicrab/commit/35fe330e66245e1addd1700d3830854763e4941a))
- bump rmcp from 1.1.0 to 1.2.0 ([f6c2cf7](https://github.com/oxicrab/oxicrab/commit/f6c2cf7dcce150b94fe9dc39adbd102964987c58))
- bump tracing-subscriber from 0.3.22 to 0.3.23 ([14b3910](https://github.com/oxicrab/oxicrab/commit/14b3910fb4398d95baae04374f5bddc2bca5f6f5))
- bump whisper-rs from 0.15.1 to 0.16.0 ([5500596](https://github.com/oxicrab/oxicrab/commit/55005967f125de94e4a942e174efa38a987b56fa))
- bump clap_complete from 4.5.66 to 4.6.0 ([7430c8a](https://github.com/oxicrab/oxicrab/commit/7430c8aa227a4b0550af3dfc984e233d24e68dc4))


### Documentation
- **build:** regenerate site pages via python docs build ([20a05a9](https://github.com/oxicrab/oxicrab/commit/20a05a9ec42428931e8c43768e0482a027a0daa0))
- **router:** refresh _pages router semantics and tool counts; stabilize arg-validation test ([06838ae](https://github.com/oxicrab/oxicrab/commit/06838ae63ce88dd0b0b93223c75c756bc85028b6))
- **architecture:** align router and execution docs with current implementation ([748ac6c](https://github.com/oxicrab/oxicrab/commit/748ac6c9133974214f7afd9487a5bfa84b219e54))
- comprehensive documentation update for message router and recent changes ([bea8d56](https://github.com/oxicrab/oxicrab/commit/bea8d5617807622e09ecfe7e7f3de9b756802596))
- document LinTS covariance degradation and fallback behavior ([ad8cc31](https://github.com/oxicrab/oxicrab/commit/ad8cc31aeb5ed99e18b269f7b095d406fcf49646))
- fix stale references and organize CLAUDE.md sections ([6c64c85](https://github.com/oxicrab/oxicrab/commit/6c64c85cdcc4c86baec6d51223b2c4cdafaf5a2b))
- document intentional channel bypass of MessageBus publish ([cebe0a7](https://github.com/oxicrab/oxicrab/commit/cebe0a73e87a65ab527bd14fad791581c4bc58d0))
- document message router, remove deleted module references ([e17b985](https://github.com/oxicrab/oxicrab/commit/e17b985c57e8a55b0b60776a4c0bd8080b35481e))
- add message router implementation plan ([615e89d](https://github.com/oxicrab/oxicrab/commit/615e89d430e63741d188d9e3139619c95ee3cdd4))
- add tool use examples to router spec ([0ce9e80](https://github.com/oxicrab/oxicrab/commit/0ce9e800e5d5c3278f31b71744e2ff3deba2a9d7))
- add semantic filtering, research context, and crate recommendations ([80f8161](https://github.com/oxicrab/oxicrab/commit/80f8161d20a93e008e9af5baaa3882d91accea25))
- add message router design spec ([486ab77](https://github.com/oxicrab/oxicrab/commit/486ab77a6b94ec9bb247214d4aae2fdd5082b2e0))
- add action dispatch implementation plan ([e0595f1](https://github.com/oxicrab/oxicrab/commit/e0595f1b032836e6d38781af527b5581a09f3134))
- add action dispatch design spec ([ccbdb21](https://github.com/oxicrab/oxicrab/commit/ccbdb216b90002ca828454326277f6cf1274eed2))
- **rss:** add getting started quick-start guide ([2f3ffde](https://github.com/oxicrab/oxicrab/commit/2f3ffdea3a0479a4209b45afbe91c4e46c79b6e7))
- **rss:** fix model terminology, add all config fields ([fce6614](https://github.com/oxicrab/oxicrab/commit/fce66147fc5cc39a4e29bc689cdc265c78b6766f))
- **rss:** fix LinTS description — Bayesian linear regression, not logistic ([995aca8](https://github.com/oxicrab/oxicrab/commit/995aca81e1c31ef19099d71d730d8dd8b769cf8c))
- **rss:** document intentional global URL dedup in scanner ([94f8c1c](https://github.com/oxicrab/oxicrab/commit/94f8c1c385f79d8945a5ce70a65e144ad8f97b7a))
- add RSS tool documentation ([1a16a6c](https://github.com/oxicrab/oxicrab/commit/1a16a6c550e24ce8b39e69c0116e09eca6ce70b1))
- add RSS tool implementation plan ([32412ac](https://github.com/oxicrab/oxicrab/commit/32412ac6213d19f6719a90a1a69e2ee2226541fa))
- add RSS tool design spec ([3bf9562](https://github.com/oxicrab/oxicrab/commit/3bf9562a5803a7d58153418468e84132b5c105fc))
- document tool metadata sideband and auto-buttons in system prompt ([6794efa](https://github.com/oxicrab/oxicrab/commit/6794efa03ada8cbd72586ed56df6b1a4d9e00e6a))
- add tool metadata sideband & auto-buttons implementation plan ([4b8e8ca](https://github.com/oxicrab/oxicrab/commit/4b8e8ca89607089ee547e595b8664fa213ba0591))


### Fixed
- **metrics:** exclude memory from semantic proxy quality and track candidate scores ([1b3fb85](https://github.com/oxicrab/oxicrab/commit/1b3fb8528d8a8ce9bec6285195d9360a5fb11f94))
- **router:** keep semantic filter narrow and align score metrics ([a9fd7f9](https://github.com/oxicrab/oxicrab/commit/a9fd7f98c89ddd34439f208433d3239f749f1dc0))
- **router:** avoid semantic metric double-count and keep helpers in semantic policy ([f04aec1](https://github.com/oxicrab/oxicrab/commit/f04aec1d72edc3a79afe5c19996c8062706372b7))
- Fix semantic threshold fallback and preserve multi-tool directives ([11dcf0b](https://github.com/oxicrab/oxicrab/commit/11dcf0b24ccedc9b93974e47fc425e621bee341f))
- **deps:** remove unmaintained backoff and update wasm/js lockfile chain ([97b93ac](https://github.com/oxicrab/oxicrab/commit/97b93ac8cbf3fb1076a28b9e8f5d07b0aaa8b0c9))
- **dispatch:** handle display_text metadata in direct dispatch path ([776feab](https://github.com/oxicrab/oxicrab/commit/776feab4009d7b08863312f4c6d10e2f3dd44f98))
- revert exfiltration guard to disabled by default ([969d1cf](https://github.com/oxicrab/oxicrab/commit/969d1cf0ec151963852e2e34b3f3e2c2753ecf71))
- restore anti-hallucination tool awareness instruction in system prompt ([32db072](https://github.com/oxicrab/oxicrab/commit/32db072f2851a736676b21559d4f6e933d59e31a))
- **router:** only route to GuidedLLM when context has live directives ([ef66c6c](https://github.com/oxicrab/oxicrab/commit/ef66c6c02a10065b12f84db2f387110477eecf02))
- **router:** don't filter tools in GuidedLLM path — use hint only ([cbed222](https://github.com/oxicrab/oxicrab/commit/cbed2220d7dde6eefef6b5fd53f3b95a5a07d50e))
- restore redirect following in default_http_client for trusted API calls ([c3753ec](https://github.com/oxicrab/oxicrab/commit/c3753ec751c188094b284a08e75c720522ca1e7d))
- **router:** prevent $N double-substitution in config rule $* expansion ([d1d9d43](https://github.com/oxicrab/oxicrab/commit/d1d9d43f61fe59cb2ba37b514d253647f5b36900))
- **test:** update config validation tests for enabled-by-default security guards ([c4ee23b](https://github.com/oxicrab/oxicrab/commit/c4ee23b07b91fe11bfed30c382b62758cf4a4914))
- **security:** add prompt guard scanning to display_text metadata ([7567cb5](https://github.com/oxicrab/oxicrab/commit/7567cb558ca99d97c523cf755623dbd4dd385d6d))
- **security:** tighten few_shot_prefix pattern to full-message start ([ef41438](https://github.com/oxicrab/oxicrab/commit/ef41438a20e4787c1b62c2b673c9168d3d506986))
- **router:** document lossy DispatchSource mapping with TODO ([bec910c](https://github.com/oxicrab/oxicrab/commit/bec910c84f1b34eb3d5b4037ed2f66143fb927c3))
- **test:** update Slack retryable test for rate limit retry ([aeb013c](https://github.com/oxicrab/oxicrab/commit/aeb013c1965b5d2cfbf6fcf1378840bc5a79d78b))
- **test:** update integration tests for async cleanup_old_sessions ([bc95f24](https://github.com/oxicrab/oxicrab/commit/bc95f240f6e47706cc12e00e629408822e80c176))
- resolve pre-existing clippy warnings ([4a27697](https://github.com/oxicrab/oxicrab/commit/4a276976f9d62cf5be3047ce38f958e204c3fd6e))
- **oauth:** fix operator precedence in expires_at computation ([9388893](https://github.com/oxicrab/oxicrab/commit/938889370ee5c60b700ac64be1e265f87c987767))
- **db:** clean up orphaned memory_sources after entry purge ([67a88be](https://github.com/oxicrab/oxicrab/commit/67a88beb2dcf9436069f04d0d51f0aeaf473c621))
- **session:** invalidate cache after cleanup_old_sessions ([433ddb4](https://github.com/oxicrab/oxicrab/commit/433ddb436af331e413352d3b52015a91ea1b8f9a))
- **circuit-breaker:** enforce minimum half_open_probes of 1 ([8aa814f](https://github.com/oxicrab/oxicrab/commit/8aa814f2d3a7de560a3439d908f5ede670785ada))
- **db:** add ESCAPE clause to workspace LIKE filters ([e5a8876](https://github.com/oxicrab/oxicrab/commit/e5a887644f099bed4b41be1836e72e384b0514ad))
- **security:** add URL-decoded redaction to leak detector ([5ad3fe8](https://github.com/oxicrab/oxicrab/commit/5ad3fe896c25789d63ce6cc15ff0b0448e78748e))
- **slack:** send error response to clear thinking emoji on processing failure ([4d1d589](https://github.com/oxicrab/oxicrab/commit/4d1d589c0fbb70aed956ef513b00b3362b37ba64))
- **slack:** send buttons even when content is empty ([0f5ba10](https://github.com/oxicrab/oxicrab/commit/0f5ba108019bfa013c606f6e9bb0f115cf182657))
- **router:** accumulate directives from all tools in multi-tool turns ([425d40d](https://github.com/oxicrab/oxicrab/commit/425d40d9e83c7f8377d142bad56c31482cc13f51))
- **router:** anchor Pattern triggers to prevent partial matches ([c2c1422](https://github.com/oxicrab/oxicrab/commit/c2c1422b2453ded47dfdd909009c563eee700b6d))
- **router:** process $N substitutions in descending order to prevent collision ([56659ce](https://github.com/oxicrab/oxicrab/commit/56659ce555c7bd3073e3d9bf5074886d69082584))
- lock on session_key in process_direct_with_overrides ([eea377d](https://github.com/oxicrab/oxicrab/commit/eea377dda78738fb5e1b5af4d9d32dfa7f34cfb2))
- lock target session in process_system_message to prevent data race ([0ae5ef5](https://github.com/oxicrab/oxicrab/commit/0ae5ef5e01a70318817d351bd92ba11097459951))
- **router:** make config command lookup case-insensitive ([63a8595](https://github.com/oxicrab/oxicrab/commit/63a8595c5d710c98fa85037317788c6283975f46))
- merge response_metadata in process_system_message outbound ([555fe7d](https://github.com/oxicrab/oxicrab/commit/555fe7df20a10927539ff7cd7dc3269d33213a86))
- re-apply tool filter after deferred tool activation ([a2c3742](https://github.com/oxicrab/oxicrab/commit/a2c37421323a23fbb69f7292f7a819f9eb2590b8))
- use floor_char_boundary for button context truncation ([aef9860](https://github.com/oxicrab/oxicrab/commit/aef9860ff23326f47ccdb4cd1f0e27e12972a31c))
- **db:** wrap 3 more multi-statement operations in transactions ([f1c6ff0](https://github.com/oxicrab/oxicrab/commit/f1c6ff05a890f887321bf99d22d1cf20e2d0b841))
- **test:** update tests for security fixes ([8723751](https://github.com/oxicrab/oxicrab/commit/8723751cc4980ddb5a69e4741231b5f10519921e))
- **security:** add URL-decoded scanning to leak detector ([8ff6f9a](https://github.com/oxicrab/oxicrab/commit/8ff6f9a8fac59e08c191c8df77f8747e0b4cdbf0))
- **security:** add prompt guard patterns for few-shot, persona, and encoded attacks ([3eec554](https://github.com/oxicrab/oxicrab/commit/3eec55437033247ed766fdc1ced5dd9f830376bf))
- **security:** add homoglyph transliteration to prompt guard normalization ([5fc2591](https://github.com/oxicrab/oxicrab/commit/5fc2591d043d48bd59dd3094f8f1350ea043d422))
- **security:** batch security hardening (10 issues) ([a0e7760](https://github.com/oxicrab/oxicrab/commit/a0e7760d37be1cfd957e7102c2dd2c1871b1ad9d))
- **security:** validate directive tool names match producing tool ([69b2e81](https://github.com/oxicrab/oxicrab/commit/69b2e813d2f347b497558f69d510d5ab2e4704e1))
- **security:** scan display_text metadata through leak detector ([87a0e8f](https://github.com/oxicrab/oxicrab/commit/87a0e8f86ad5557682ed45b9b1d1411f628e9563))
- **security:** sanitize error messages in direct dispatch paths ([f4171fc](https://github.com/oxicrab/oxicrab/commit/f4171fce21e46d755e7fc0146a55b786050206a4))
- **security:** add secret scanning and prompt guard to process_system_message ([0a1b5d6](https://github.com/oxicrab/oxicrab/commit/0a1b5d6d3e9471e648f45c66157ddeff1dcf4406))
- **security:** enable prompt guard (warn) and exfiltration guard by default ([0d65d76](https://github.com/oxicrab/oxicrab/commit/0d65d76af7a76d98affe3eaa85ee979ee1bdc257))
- **security:** disable HTTP redirects in RSS pinned clients to prevent SSRF ([1a92599](https://github.com/oxicrab/oxicrab/commit/1a9259917194c2e4f09c093bedf21c43f0235ce1))
- **db:** convert cron transactions and add memory purge test ([658ba8c](https://github.com/oxicrab/oxicrab/commit/658ba8c0c08843eb8293a21f07eb42560b3a451e))
- evict stale status message tracking maps ([9484dfa](https://github.com/oxicrab/oxicrab/commit/9484dfad90ef1ec11025d335de3cae3b7b1f1eef))
- **provider:** add session affinity header to OAuth warmup request ([1c2d98e](https://github.com/oxicrab/oxicrab/commit/1c2d98e8b999ccd60d0dc1d4b5673aff8ee56b24))
- **db:** add memory entry purge to prevent unbounded growth ([9674741](https://github.com/oxicrab/oxicrab/commit/9674741305d67fa3ff96878c1e85c3b2ebb4b13b))
- **db:** add purge for intent_metrics table ([884f38d](https://github.com/oxicrab/oxicrab/commit/884f38d8d36b7635a859b6cac5304ddd34aea9ee))
- **db:** wrap insert_memory in transaction for atomicity ([49b0fb9](https://github.com/oxicrab/oxicrab/commit/49b0fb9c43f7d8036753044d07ce377e34b220be))
- **test:** update hallucination tests for user message correction format ([ee6660a](https://github.com/oxicrab/oxicrab/commit/ee6660a228720140651cdd84cee92906e4c08335))
- hallucination correction, directive consumption order, gmail reply button ([21aec49](https://github.com/oxicrab/oxicrab/commit/21aec491ef6d8dce5b1d0a9b8b373781c3d23bac))
- **test:** normalize trigger in case-insensitive test ([c280de3](https://github.com/oxicrab/oxicrab/commit/c280de36b100d1295cec58d616191e216d740062))
- **router:** address review findings — dispatch store, context hint, TOCTOU, param fixes ([6e5be2b](https://github.com/oxicrab/oxicrab/commit/6e5be2ba8fc5e07200ce305edef036723902043f))
- **google_calendar:** add rsvp action for button dispatch ([730be1b](https://github.com/oxicrab/oxicrab/commit/730be1b6779b749237c374208e9f05158869a4bf))
- **cron:** add pause and resume actions for button dispatch ([0c5e8b2](https://github.com/oxicrab/oxicrab/commit/0c5e8b20d5d503de6b591f803e777ea5f61bb10b))
- **github:** use correct action names and params in button contexts ([cb364c9](https://github.com/oxicrab/oxicrab/commit/cb364c91e3ad13f283f874584bfe497fb8f4f2c8))
- **google_tasks:** correct task_list_id param name in button context ([dd37da6](https://github.com/oxicrab/oxicrab/commit/dd37da6d5e41e6a54165db5d69ca63412d842198))
- **security:** prevent JSON injection in config rule substitution ([ec0c2e2](https://github.com/oxicrab/oxicrab/commit/ec0c2e2813dbd7512655fef9393fd06b1b976653))
- **security:** close secret scanning and prompt guard gaps in direct dispatch ([9e596e5](https://github.com/oxicrab/oxicrab/commit/9e596e564b6c466dbf5cfb66cb32d52860245751))
- **rss:** bypass LLM summarization with display_text passthrough ([5c7c3ff](https://github.com/oxicrab/oxicrab/commit/5c7c3ff2eae1db5446293df951056a7091da80c8))
- **rss:** rename 'next' action to 'review' for direct lexical match ([313916d](https://github.com/oxicrab/oxicrab/commit/313916d38a135af63766273bee351322313d7710))
- **tools:** add dispatch guidance to action descriptions across all tools ([9555a56](https://github.com/oxicrab/oxicrab/commit/9555a56bfd7226db60860e95a18d91ea7e041a55))
- **rss:** guide LLM to use 'next' action for article review workflow ([19016b2](https://github.com/oxicrab/oxicrab/commit/19016b21234037c891240f13a02c242da8e7534f))
- **hallucination:** reject vapid short questions and deduplicate buttons by label ([f1cd3d9](https://github.com/oxicrab/oxicrab/commit/f1cd3d96f9e004be932cc5324f51c2507d6f0da5))
- **hallucination:** catch button clicks and missing action verbs ([d24e780](https://github.com/oxicrab/oxicrab/commit/d24e7806d6b7af99930987e50075c86df06c493a))
- **hallucination:** expand intent classifier with 27 missing action verbs ([f862a16](https://github.com/oxicrab/oxicrab/commit/f862a16c78dde120cb7f9232fbbb2dd3bada8a14))
- **rss:** include explicit tool call instructions in button context ([76d7823](https://github.com/oxicrab/oxicrab/commit/76d78234a4a2b36fdcf0171800a1ba0742c0e610))
- **rss:** enforce individual article presentation with buttons ([3a31faf](https://github.com/oxicrab/oxicrab/commit/3a31fafe339b08a28fb820f4bb6fca949f2f5661))
- pass per-provider temperature to compactor when routing overrides provider ([61999bc](https://github.com/oxicrab/oxicrab/commit/61999bc2b66334d0f110146fab09ee42a2c68121))
- **rss:** skip model inflation on empty scans, tighten cron TOCTOU guard ([53c6160](https://github.com/oxicrab/oxicrab/commit/53c616034f983fcd9322e83f7d0788cb63181ebf))
- **rss:** skip IPv6 addresses when IPv4 is available in pinned clients ([7fb4954](https://github.com/oxicrab/oxicrab/commit/7fb4954760af4eb877d245338476c4c097a515b6))
- **rss:** add User-Agent to pinned HTTP clients ([a3133a0](https://github.com/oxicrab/oxicrab/commit/a3133a0cce45221ff79224fb8ff7bf1076f17bda))
- **rss:** purge all stale articles, not just unreviewed ones ([e29071b](https://github.com/oxicrab/oxicrab/commit/e29071b53cea4546268b71a00b9d64e431d4a902))
- **rss:** cap limit and offset to prevent oversized rank windows ([3ed714a](https://github.com/oxicrab/oxicrab/commit/3ed714aaa26e96dabb24fca88b3a1a14ae654ad4))
- **rss:** use checked i64 conversion in now_ms ([035dadb](https://github.com/oxicrab/oxicrab/commit/035dadb46e44735d7409a964568ae6da9f5a2ff0))
- **rss:** chunk batch tag query to avoid SQLite variable limit ([8310693](https://github.com/oxicrab/oxicrab/commit/8310693a1bb25f077e07129b7030267e84520ba4))
- **rss:** word-boundary for rust keyword, feed_id description, stale comment, dup doc ([62641b6](https://github.com/oxicrab/oxicrab/commit/62641b62d4e3c0ee84a846260100fdf06f068b18))
- **rss:** use word-boundary matching for 'ai' keyword in feed suggestions ([84a2818](https://github.com/oxicrab/oxicrab/commit/84a281810d8bb5d2decc49b9c27f96cda49d3472))
- **rss:** resolve short feed IDs in get_articles, sync docs ([e35993a](https://github.com/oxicrab/oxicrab/commit/e35993a28499ebc526b8311f2d9a3fc92d2dd47c))
- **rss:** add feed ID resolution, enable_feed action, fix short ID lookup ([6157953](https://github.com/oxicrab/oxicrab/commit/6157953b04d1af40de755ecdce9575dbdb45cfdb))
- **rss:** remove dead "skipped" status, batch tag queries in ranking ([b35aecd](https://github.com/oxicrab/oxicrab/commit/b35aecd101a7ab00584f1324dc70db6e07058911))
- **rss:** include keywords in browse ranking, use char count for profile validation ([cfdea12](https://github.com/oxicrab/oxicrab/commit/cfdea12b7ebd93566116e0f18f883ae15213665e))
- **rss:** allow scan during calibration, fix feed summary key collision ([9189b1f](https://github.com/oxicrab/oxicrab/commit/9189b1f3c7367673057ccf7e7e820ce9104c4ae3))
- **rss:** atomic cron registration, fix snippet truncation for multibyte ([66efc03](https://github.com/oxicrab/oxicrab/commit/66efc03d11f41206a1ee38e94afa0904fb72780b))
- **rss:** configurable ingest window, keywords in feedback, cron recovery msg ([c12463c](https://github.com/oxicrab/oxicrab/commit/c12463c0e28370d2f3d6eb62e44aa9abc4fca94e))
- **rss:** expand rank window to cover requested page offset ([b774e9e](https://github.com/oxicrab/oxicrab/commit/b774e9e6fa5c36b815ccb3d5f956cdaee6270d5d))
- **rss:** use 0 not i64::MAX as purge cutoff fallback ([053b53e](https://github.com/oxicrab/oxicrab/commit/053b53eac0a75f878af6c47c533a9d039dbacbb9))
- **rss:** accurate pagination count, HTTPS for arXiv, short IDs in calibration ([d2df4bb](https://github.com/oxicrab/oxicrab/commit/d2df4bbcd3be38ab17ab34ceff2547c90a09c38d))
- **rss:** prevent vector dimension panic, fix LIKE escaping, widen ranking window ([e379884](https://github.com/oxicrab/oxicrab/commit/e379884972abc9e6bf1c91f5b0c63c421271d063))
- **rss:** cap feed body reads at 10MB to prevent OOM ([718b7f4](https://github.com/oxicrab/oxicrab/commit/718b7f4a22eb0522b3f617ced818fadea7ad5a1c))
- **rss:** address second round of PR feedback ([685ed2b](https://github.com/oxicrab/oxicrab/commit/685ed2b75b4023b62d6247a324d9861dcc004a92))
- **rss:** address PR review — single TS draw, fix tool count ([ffae116](https://github.com/oxicrab/oxicrab/commit/ffae1164c36321345aeeacfe607137b43a140816))
- **rss:** address code review findings (SSRF, schema types, calibration status) ([a4bea59](https://github.com/oxicrab/oxicrab/commit/a4bea59da006502b34dfd12222a7040f80070c41))
- **hallucination:** add Layer 3 action gap detection for partial hallucinations ([71bf72d](https://github.com/oxicrab/oxicrab/commit/71bf72d52e20c775da964d699c99378411c10786))
- **hallucination:** detect present-tense and intent action claims ([81d2956](https://github.com/oxicrab/oxicrab/commit/81d2956c2855ca753a4bb0d1d3e3971761211cfb))
- **google_mail:** improve HTML body extraction for marketing emails ([fe88550](https://github.com/oxicrab/oxicrab/commit/fe88550fdf2fecf6111bd007ac7427b74ac57c68))
- **slack:** add thinking emoji on button click ([709ed2a](https://github.com/oxicrab/oxicrab/commit/709ed2ae69e27b6efd0547648e2dc8adf36a843c))


### Maintenance
- **clippy:** enforce warning-free lint baseline across router and loop ([fd082d0](https://github.com/oxicrab/oxicrab/commit/fd082d0a306d934e17f35097b6179865ffee1c9b))
- **rss:** remove dead disable_rss_feed method ([0df3aea](https://github.com/oxicrab/oxicrab/commit/0df3aea76fa313535d40116ca70a65518cca7599))
- add feed-rs, nalgebra, rand_distr deps for RSS tool ([100328e](https://github.com/oxicrab/oxicrab/commit/100328ec142000a83b4ceb929825a68e414adf56))


### Other
- Create CNAME ([e9f2895](https://github.com/oxicrab/oxicrab/commit/e9f2895b2fed24639fa6e3366fb2ac8dd121b0c9))
- Create CNAME ([4ec1f5c](https://github.com/oxicrab/oxicrab/commit/4ec1f5c8bb6375557d4e6d196309679704fd91cf))


### Performance
- **router:** cache compiled Pattern regexes in LRU to avoid recompilation ([e0cf8e5](https://github.com/oxicrab/oxicrab/commit/e0cf8e5700322ace9d4c48c4661f49ce288a9727))
- pre-lowercase message once in route() for all matching ([c9604fb](https://github.com/oxicrab/oxicrab/commit/c9604fb5473348b20328da89755c52bd9953c14e))
- **db:** use timestamp directly instead of DATE() to enable index usage ([4386bf5](https://github.com/oxicrab/oxicrab/commit/4386bf5769497da3033caad0c961f8bb10392548))
- **gateway:** optimize startup by removing blocking actions (#91) ([0da0e48](https://github.com/oxicrab/oxicrab/commit/0da0e487505657f7146291309bc0132b457d5501))


### Removed
- Removed specs ([ae85e5b](https://github.com/oxicrab/oxicrab/commit/ae85e5bbce7033318b264f88986d1614dab586b5))
- Removed specs ([7563462](https://github.com/oxicrab/oxicrab/commit/756346261e38e7ea70707b94876ccb3d6aefa843))
- Removed docs ([c5c9b61](https://github.com/oxicrab/oxicrab/commit/c5c9b61eb309a2202a684b16ded3bc6701a6c0a5))
- removed research ([fc7ffc5](https://github.com/oxicrab/oxicrab/commit/fc7ffc5d85d35e0c4ea0871ef4f7eb5ed9eb06ae))


### Security
- Perf/audit fixes (#93) ([e1479e8](https://github.com/oxicrab/oxicrab/commit/e1479e81fd3f2ea132e042207fd054974b7cb58c))


### Testing
- **router:** add integration tests for message router ([c0750c9](https://github.com/oxicrab/oxicrab/commit/c0750c9c69d0241182e259cf76b4a7c18464c1f1))
- **rss:** add full onboarding integration test ([8b150f1](https://github.com/oxicrab/oxicrab/commit/8b150f11c0c2916d207e036d754d0821adc97434))

## [0.14.5] - 2026-03-11

### Added
- **skills:** add security scanner for skill files before injection ([ef48590](https://github.com/oxicrab/oxicrab/commit/ef48590d4ae225c32d4224b0d2833f2e989da499))
- **cron:** add delay_seconds for relative one-shot scheduling ([faa2c2c](https://github.com/oxicrab/oxicrab/commit/faa2c2c20ee58ff64c4b3123f61e529492cb5cac))


### CI/CD
- bump docker/setup-buildx-action from 3 to 4 (#76) ([4debc80](https://github.com/oxicrab/oxicrab/commit/4debc80a3184b3c5c77c4bf0096afcb8dfd094cd))
- bump docker/metadata-action from 5 to 6 (#77) ([c022d27](https://github.com/oxicrab/oxicrab/commit/c022d270d415f2af909357d903e4652265315559))
- bump docker/build-push-action from 6 to 7 (#78) ([0b032f1](https://github.com/oxicrab/oxicrab/commit/0b032f1810cb2ef3cde55a4b49be9bbdab1cbdc0))
- bump trufflesecurity/trufflehog from 3.88.26 to 3.93.7 (#79) ([ac95b5c](https://github.com/oxicrab/oxicrab/commit/ac95b5c870c646acea034d68b04777d56c990840))
- bump docker/login-action from 3 to 4 (#80) ([4d16a08](https://github.com/oxicrab/oxicrab/commit/4d16a080db1ee431224f3f02b7bde939c8687109))


### Dependencies
- bump which from 8.0.0 to 8.0.1 (#81) ([77317e6](https://github.com/oxicrab/oxicrab/commit/77317e608bc47b8c856195e656d0ebfdca0b8a13))
- bump tokio from 1.49.0 to 1.50.0 (#82) ([8eabf09](https://github.com/oxicrab/oxicrab/commit/8eabf096da96cf932b455633092fabcb0264e0b1))
- bump uuid from 1.21.0 to 1.22.0 (#84) ([5855659](https://github.com/oxicrab/oxicrab/commit/58556590abecd8bb2e5704f655f8cf8a2f5ea420))
- bump rustls from 0.23.36 to 0.23.37 (#85) ([f724372](https://github.com/oxicrab/oxicrab/commit/f7243728ec4913b6aa8fc50e7c77fee7a8decaa4))


### Documentation
- update for recent features (skill scanner, cron delay, MCP fixes) ([a1420a8](https://github.com/oxicrab/oxicrab/commit/a1420a8b126e4369bd066f945bb5dc43f7aaab7b))


### Fixed
- **buttons:** clippy fixes, ID validation, cron metadata forwarding, tool hints (#89) ([60585aa](https://github.com/oxicrab/oxicrab/commit/60585aa079f2c8672eef995981b060fe0797236e))
- **mcp:** strip null params and reject CRLF in env vars ([34bae09](https://github.com/oxicrab/oxicrab/commit/34bae09016a54f852fc644782729098803f83483))


### Maintenance
- **deps:** update quinn-proto 0.11.13 → 0.11.14 (CVE fix) ([b4c630d](https://github.com/oxicrab/oxicrab/commit/b4c630d38305c61db7bab43931aec0542c8865ea))

## [0.14.4] - 2026-03-09

### Changed
- Revert "fix(db): wrap DLQ insert+purge in transaction to prevent TOCTOU" ([bb4dfcd](https://github.com/oxicrab/oxicrab/commit/bb4dfcda23f7da24e97e92c2c736467861821270))


### Fixed
- **loop:** persist reasoning_content to session history ([75e80ed](https://github.com/oxicrab/oxicrab/commit/75e80ed81fde45b4372133f30f6ffc56d79d74a6))
- address medium-severity audit findings across subsystems ([2b49974](https://github.com/oxicrab/oxicrab/commit/2b499747a007c299fa94e9795e424669c76ca781))
- **discord:** check interaction token TTL before followup ([d47a38d](https://github.com/oxicrab/oxicrab/commit/d47a38d8d3ce0e3849855f3afa152b0baacb6692))
- **db:** wrap DLQ insert+purge in transaction to prevent TOCTOU ([c11e576](https://github.com/oxicrab/oxicrab/commit/c11e5764503760b694d61bf03b9dbac8b8ec5cb4))
- **loop:** prevent wrapup hint from firing on final iteration ([60de312](https://github.com/oxicrab/oxicrab/commit/60de312917fa8424259d68a07831fba72a1d4432))
- Docker build, startup robustness, and contributor credit ([f34af0c](https://github.com/oxicrab/oxicrab/commit/f34af0cb49852c838994562d757ddb0075a24e61))
- **slack:** strip think tags on all exit paths and improve table conversion ([6649edb](https://github.com/oxicrab/oxicrab/commit/6649edbdbafb877305d0bb826c306df2a6f381b6))


### Performance
- **providers:** use Arc<Vec<ToolDefinition>> in ChatRequest to avoid cloning per iteration ([955efe7](https://github.com/oxicrab/oxicrab/commit/955efe7df9b8defe8a6395baf845654016effa21))
- **loop:** use Aho-Corasick for tool mention detection ([63927bb](https://github.com/oxicrab/oxicrab/commit/63927bbd0490db517334aed4c55dcef654554d79))
- **tools:** cache tool definitions at registration time ([834e164](https://github.com/oxicrab/oxicrab/commit/834e164750d75d3b257437591451764848de354a))
- **slack:** cache mention regex instead of recompiling per message ([508f85d](https://github.com/oxicrab/oxicrab/commit/508f85d5bc988249c4f8f04c85a5a867aecb7809))
- **cron:** cache compiled regexes in event matcher ([72e0e12](https://github.com/oxicrab/oxicrab/commit/72e0e124dfbd6b3b16e600430df8273d4f4fa8c0))
- **safety:** LazyLock prompt guard patterns, fix double normalization ([0212391](https://github.com/oxicrab/oxicrab/commit/02123915951c14e9cdc51c2f045ec72b1b10f01c))
- **db:** add index on memory_entries(source_key, created_at) ([9f192e6](https://github.com/oxicrab/oxicrab/commit/9f192e6ac07763fa2b3bb3afbeaf4b0ef3085edd))


### Testing
- add coverage for google API client and MCP env var detection ([1d9855c](https://github.com/oxicrab/oxicrab/commit/1d9855c2c65ef5339007ba1671073b13bd2ef186))
- **cli:** add command parsing and validation tests ([3327b89](https://github.com/oxicrab/oxicrab/commit/3327b8971a9ada93384fecdd48a9c9fade1b20f5))
- **config:** add routing resolution tests ([2e402df](https://github.com/oxicrab/oxicrab/commit/2e402dfd798c6638c16a24621449b258faa62258))

## [0.14.3] - 2026-03-08

### Added
- **docker:** add slack-only image variant and fix cfg gates ([bd48dd8](https://github.com/oxicrab/oxicrab/commit/bd48dd8f75ec2de4bc830a007607cd5dc9d636dd))


### Documentation
- round 3 audit fixes — cross-doc consistency and fabricated config ([3c83375](https://github.com/oxicrab/oxicrab/commit/3c83375d5ee763676eb958d5c2acbee504b801dc))
- fix remaining issues from second audit pass ([3dc9712](https://github.com/oxicrab/oxicrab/commit/3dc9712c9af2bbc42d4173288c2a09d99299495c))
- comprehensive audit and fix across all documentation ([0f60f7c](https://github.com/oxicrab/oxicrab/commit/0f60f7cbc5beb4801e4cce5a822bdef6cbb2a3be))


### Fixed
- **docs:** replace remaining ghost model field in config.html examples ([8740014](https://github.com/oxicrab/oxicrab/commit/8740014d17fc701022745e227db7f1b1b6b7bbc3))
- **slack:** convert markdown tables to text and strip <think> tags ([8e121ff](https://github.com/oxicrab/oxicrab/commit/8e121ff6d86d310253e12f0728060c0393417f75))


### Maintenance
- **deps:** upgrade rmcp 0.17→1.1, chromiumoxide 0.8→0.9, governor 0.8→0.10 ([e3f54fe](https://github.com/oxicrab/oxicrab/commit/e3f54fe704aeb667276528837936a68b2dda9b31))
- **deps:** upgrade rmcp 0.17→1.1, chromiumoxide 0.8→0.9, rusqlite 0.37→0.38, governor 0.8→0.10 ([986823c](https://github.com/oxicrab/oxicrab/commit/986823cc47d2ed74e3ea144a6f6ad82f57746af5))
- **deps:** switch whatsapp-rust from git ref to crates.io 0.3.0 ([76ad42c](https://github.com/oxicrab/oxicrab/commit/76ad42c0745491638fa2b9a0d5a6a4753ce8d7ea))

## [0.14.2] - 2026-03-07

### Added
- add param auto-casting, schema hints on errors, and finish_reason guard ([a43e6d6](https://github.com/oxicrab/oxicrab/commit/a43e6d606dfa1479c20ea5c49ebde59271e3745f))
- add tool output stash and improve datetime prominence in system prompt ([292411f](https://github.com/oxicrab/oxicrab/commit/292411f0645fb34fc36501e449e18117b58f5168))
- add cron self-scheduling guard, process group kill, deferred tool registry, and session affinity ([523ee50](https://github.com/oxicrab/oxicrab/commit/523ee5059c01de12fc40965c4c6b0d9942c41aea))
- **tools:** add google_tasks tool with 6 actions ([942a980](https://github.com/oxicrab/oxicrab/commit/942a980c7f29d9edaab1c2039f1058ab05521102))


### Changed
- **config:** replace Google scopes with per-tool enable flags ([741a5a4](https://github.com/oxicrab/oxicrab/commit/741a5a433cff3e6d40eab6d7db73e4e63861ea45))
- **db:** switch memory schema bootstrap to versioned migrations ([5f3790b](https://github.com/oxicrab/oxicrab/commit/5f3790bbbe020d6a9ee677b8472b0b50d820b4ab))
- **auth:** share oauth credential persistence and safe file I/O ([927872b](https://github.com/oxicrab/oxicrab/commit/927872b3e6b81edb07395ff084a76d2ef8643f57))


### Documentation
- **deploy:** add docker auth and scope re-authentication instructions ([e96515a](https://github.com/oxicrab/oxicrab/commit/e96515a274b771cd6cfcd1b9b251085cae48323d))
- add stash_retrieve tool and datetime prominence to docs ([57ff872](https://github.com/oxicrab/oxicrab/commit/57ff872714e8ebc05dbd13a108ebf67d9b016a9d))
- redesign with warm crafted theme ([de7856f](https://github.com/oxicrab/oxicrab/commit/de7856f27b9795209be89301a31e0456598f38e9))


### Fixed
- **auth:** save Google OAuth credentials to database during auth flow ([cd3ce6e](https://github.com/oxicrab/oxicrab/commit/cd3ce6e4c62b86440ecaf7105167aab4eb8694e2))
- **ci:** read toolchain version from rust-toolchain.toml ([869a3f5](https://github.com/oxicrab/oxicrab/commit/869a3f59170b5e66f59eb823c2fa6b526bb1f9ae))
- **docker:** sync nightly version with rust-toolchain.toml ([b2608ba](https://github.com/oxicrab/oxicrab/commit/b2608bac9745f676f974b9b3a7a9ee83bdd225a0))
- **docs:** SVG fill attribute, reduced-motion, and noscript fallback ([efd5f71](https://github.com/oxicrab/oxicrab/commit/efd5f7132373426d47fa65fdc61a5406e5e75ceb))
- **test:** update error tool test for schema hint injection ([c329457](https://github.com/oxicrab/oxicrab/commit/c3294570246f32b749e2e779578b7d0038432c60))
- **tools:** correct todoist priority ordering and stash documentation ([efa44f9](https://github.com/oxicrab/oxicrab/commit/efa44f971b285473b7a9fc0a5a97e01374eae682))
- **shell:** pipe stdout/stderr for spawn and fix approx_constant in test ([7d8e59b](https://github.com/oxicrab/oxicrab/commit/7d8e59b43224580b08cd99af407c2414579c029e))
- **tools:** use ToolResult::error for user-facing parameter validation ([b7eea60](https://github.com/oxicrab/oxicrab/commit/b7eea6041f903648eb293d08ed14a6244ed9b102))
- harden security, capabilities, and correctness across subsystems ([c3203fc](https://github.com/oxicrab/oxicrab/commit/c3203fc44b83aa562961b9d1c3c7b25ebc34549a))
- **clippy:** simplify map_or to is_none_or with method reference ([776aa35](https://github.com/oxicrab/oxicrab/commit/776aa356422b3196568dfc6e881ae92426dbef0f))
- **tools:** reject empty update_task with no fields to modify ([5f95b11](https://github.com/oxicrab/oxicrab/commit/5f95b11a35f84b0f896e2167d67d4a37dbb2bc99))
- **test:** use RAII guard for env var cleanup and tolerate mutex poisoning ([47d106b](https://github.com/oxicrab/oxicrab/commit/47d106bd0ec26f75ae9626f7a7d7374bfa2212f7))
- **clippy:** inline format args and nest migration patterns ([03f4075](https://github.com/oxicrab/oxicrab/commit/03f4075f3e150e6a198364ba0104ce4ab55ea3eb))
- **review:** address oauth permission and migration hardening feedback ([4570e71](https://github.com/oxicrab/oxicrab/commit/4570e71eac2b2f1ffe1a50e45837c7c1ada11cdf))


### Maintenance
- read nightly version from rust-toolchain.toml everywhere ([79f8319](https://github.com/oxicrab/oxicrab/commit/79f83190804e8aba60772b605d1ae22c8bde0c95))
- update Rust nightly to 2026-03-05 and fix flaky test ([73253d2](https://github.com/oxicrab/oxicrab/commit/73253d28f8352e411bd7dfe08e0c8a2184886883))


### Removed
- removed research ([4ae5e8f](https://github.com/oxicrab/oxicrab/commit/4ae5e8f775aeb17acd187e2c791d7b8d1ea6e3a3))

## [0.14.1] - 2026-03-06

### Added
- **memory:** add explain_last action to memory_search tool ([3ca2ea2](https://github.com/oxicrab/oxicrab/commit/3ca2ea28bef2b2e38818f8ab31bdc8aff2f92fb6))


### CI/CD
- restructure pipeline for faster PRs and better security ([7127e23](https://github.com/oxicrab/oxicrab/commit/7127e237d77e4c3982f4be8d7e9735a27f3998d3))


### Fixed
- move struct definition before statements to satisfy clippy ([efdbabe](https://github.com/oxicrab/oxicrab/commit/efdbabe8499033dd8463f2866c8141a1c629a149))
- **pairing:** revert CSPRNG code generation — uuid::new_v4() hangs ([eaceed8](https://github.com/oxicrab/oxicrab/commit/eaceed88093d2b7c93c957dbf014370770544294))
- update extract_media_paths tests for media dir restriction ([fb9804f](https://github.com/oxicrab/oxicrab/commit/fb9804fdb7a40a40b2d9bcfdded759b49ac94fd6))
- **memory:** clarify query is required for search action in schema description ([75f7510](https://github.com/oxicrab/oxicrab/commit/75f75108480ebde2e1bc43257614a87a3df57f68))
- **pairing:** revert CSPRNG code generation — uuid::new_v4() hangs ([304ee49](https://github.com/oxicrab/oxicrab/commit/304ee49da57dfc8ab99b843376549e3df31084b4))
- address 3 remaining audit findings ([0d7a1a0](https://github.com/oxicrab/oxicrab/commit/0d7a1a0678989b17ca8487788761a0b74e90711b))
- restrict media path extraction to trusted media directory ([6ccdb01](https://github.com/oxicrab/oxicrab/commit/6ccdb01987cee091844a273f67bcce8632c77836))
- address 6 bugs from focused cron/memory audit ([fc384a7](https://github.com/oxicrab/oxicrab/commit/fc384a7e7737358a6a94f6b080014ec3b9c8ef3a))

## [0.14.0] - 2026-03-06

### Added
- **subagent:** migrate activity logs to MemoryDB ([97b91d2](https://github.com/oxicrab/oxicrab/commit/97b91d244c843f710e31fee0c16d5f6ae8cacc2d))
- **media:** add optional DB registration to save_media_file ([60613b1](https://github.com/oxicrab/oxicrab/commit/60613b1d2fa7f1e82758b2aaf23fad5b48a168fb))
- **obsidian:** migrate cache state to MemoryDB ([5307012](https://github.com/oxicrab/oxicrab/commit/5307012767a8685d1724b7f162042f192af08173))
- **auth:** migrate OAuth token caching to MemoryDB ([dbbc42a](https://github.com/oxicrab/oxicrab/commit/dbbc42a499b8df9d8fa1adeb47e5a81a720713aa))
- **pairing:** migrate pairing state from JSON files to MemoryDB ([aa941c6](https://github.com/oxicrab/oxicrab/commit/aa941c67ba715fd6a48965d6500eaa07a53f744f))
- **cron:** add cron_jobs tables and CRUD module to MemoryDB ([44f03e3](https://github.com/oxicrab/oxicrab/commit/44f03e39a1311d33c115dd44624a351823909ede))


### Changed
- **cron:** replace file-based CronService with MemoryDB backend ([20f2145](https://github.com/oxicrab/oxicrab/commit/20f21451f6d3c1e8a0320ee7afd826c84bdbfa2b))
- **cron:** remove origin_metadata from CronPayload ([c27b0e9](https://github.com/oxicrab/oxicrab/commit/c27b0e90be47014e12aa62981545b9435340d243))


### Documentation
- add design for 5 file-to-DB migrations ([31d38f5](https://github.com/oxicrab/oxicrab/commit/31d38f548a50d1c400f48f62a5c8b0cabc33e03a))
- update for cron SQLite migration ([ae7053b](https://github.com/oxicrab/oxicrab/commit/ae7053bec50cedcd4cb8d37b4c581ac2ef53ed0c))


### Fixed
- correct 8 stale/wrong comments and 4 stale doc references ([7668e07](https://github.com/oxicrab/oxicrab/commit/7668e0739f5b0f6cddfca3e15c990810bf677e47))
- address findings from third full codebase audit ([599c9ac](https://github.com/oxicrab/oxicrab/commit/599c9acaa5e7c83a0dc4f65b1cb9dfc9a0f62a35))
- address 8 findings from second full codebase audit ([e55d48b](https://github.com/oxicrab/oxicrab/commit/e55d48b7cf7432851af7019176db42f61794c89e))
- address remaining findings from full codebase audit ([4ae5970](https://github.com/oxicrab/oxicrab/commit/4ae597077cdf91175b560015122ba9a3613ae3c7))
- address 4 findings from full codebase security audit ([24e4542](https://github.com/oxicrab/oxicrab/commit/24e4542ec72a2905bbc66ed003d2854302ef2aa2))
- use get_oxicrab_home() consistently for all oxicrab paths ([71353e2](https://github.com/oxicrab/oxicrab/commit/71353e2108a153253a54f23f5890465d8497c221))
- add get_memory_db_path() utility for consistent DB path resolution ([e00a69f](https://github.com/oxicrab/oxicrab/commit/e00a69f8f21e4b649cf688b58d0f301cc39f6c81))
- address deep review findings ([b5a1c33](https://github.com/oxicrab/oxicrab/commit/b5a1c330734b7bbdca077855f1793b5932f03706))
- address PR review findings ([3dc4e53](https://github.com/oxicrab/oxicrab/commit/3dc4e530c7e7a98c179d28255edcaf55a414747b))
- **cron:** consolidate update_cron_job to include next_run_at_ms ([898fe5e](https://github.com/oxicrab/oxicrab/commit/898fe5e43cab42766d5e26737df146b0c6e36baa))
- **cron:** address 3 bugs from codebase audit ([276cdd4](https://github.com/oxicrab/oxicrab/commit/276cdd4e952e27aa8ebfa174688fef856842cafa))


### Maintenance
- **cron:** remove CronStore struct ([55506c9](https://github.com/oxicrab/oxicrab/commit/55506c9f2a662f331e97267f3df904647ba1e477))


### Removed
- Removed plan ([fdbc95e](https://github.com/oxicrab/oxicrab/commit/fdbc95e85730820829813e886cb5fa1eb216ca83))

## [0.13.5] - 2026-03-05

### Changed
- replace file-based memory with DB-only storage ([1aeaf08](https://github.com/oxicrab/oxicrab/commit/1aeaf087ae8d7e7045db1a0fad03458b7aaeffed))
- remove heartbeat service entirely ([2eabbd1](https://github.com/oxicrab/oxicrab/commit/2eabbd146190ffc630d97f8110b9e058444af09b))


### Fixed
- address 8 bugs from full codebase audit ([2696bf9](https://github.com/oxicrab/oxicrab/commit/2696bf9a5efa2b6864dc3e133840bb315b7bb434))
- address all memory system audit findings ([2b36aaf](https://github.com/oxicrab/oxicrab/commit/2b36aaf4933536c63f756e34201bdc1213dc579e))
- address memory system audit findings ([176bde8](https://github.com/oxicrab/oxicrab/commit/176bde8fdf79d16e32b2e9c57bf10996c8d899ed))
- create workspace template files on gateway startup ([0ac695b](https://github.com/oxicrab/oxicrab/commit/0ac695bd36f0a768c6f5449f2dd646c405ab75b8))

## [0.13.4] - 2026-03-05

### Added
- auto-download whisper GGML model when localModelPath is missing ([b42b459](https://github.com/oxicrab/oxicrab/commit/b42b459d6c41d9caf2db34bc4b1c0c9323db3b11))
- Add Contributor Covenant Code of Conduct ([6b3647d](https://github.com/oxicrab/oxicrab/commit/6b3647d5a8fac9dd1dfe596d7e3225918d29bfd6))


### Documentation
- update deploy docs for pre-built GHCR image and ~/.oxicrab default ([d357667](https://github.com/oxicrab/oxicrab/commit/d357667488cbd50a906120606d08ebb131768f13))


### Fixed
- install rustls ring crypto provider at startup ([3032b7b](https://github.com/oxicrab/oxicrab/commit/3032b7b45481295f03b44cd3c3feab76539b9018))
- add CMD prefix to docker-compose healthcheck test ([7ec76d1](https://github.com/oxicrab/oxicrab/commit/7ec76d1b86e8fc9ad49efb3c8c879a7eaf2ab28f))
- use parsed URLs for Twilio API calls to satisfy CodeQL CWE-319 ([5cdb64f](https://github.com/oxicrab/oxicrab/commit/5cdb64f507ea03c9fd1997e8d0ae2d3d468c3527))


### Maintenance
- use pre-built GHCR image instead of building from source ([eb1773c](https://github.com/oxicrab/oxicrab/commit/eb1773cf15f9b33497935fc16ccce0003cf9fa4b))

## [0.13.3] - 2026-03-04

### Added
- add minimax provider for OpenAI-compatible API ([dc469f9](https://github.com/oxicrab/oxicrab/commit/dc469f9506fca93768e484f7c84467467a69591a))
- add builder patterns for ChatRequest, InboundMessage, OutboundMessage ([a587562](https://github.com/oxicrab/oxicrab/commit/a587562481d2458c82ecf91312a58c5fe51ac387))
- **costguard:** Removed CostGuard ([1d3ed47](https://github.com/oxicrab/oxicrab/commit/1d3ed4788e6ecc88d98afd5854de7163513f351e))


### Changed
- replace fragile credential count assertions with bounds checks ([df7654d](https://github.com/oxicrab/oxicrab/commit/df7654dfa0d0adec68c9c1f818670a8bbe35beaa))


### Fixed
- address 12 bugs in subagent, tools, and cron from deep audit ([223f1a1](https://github.com/oxicrab/oxicrab/commit/223f1a1bbc9d1a6a0e81dd4236afd673c8a800ec))
- address 12 bugs found in tools audit ([ed31b28](https://github.com/oxicrab/oxicrab/commit/ed31b28c25a8fe52ae598cbc23589324627ea862))
- prefer IPv4 in DNS resolution order for pinned clients ([fecbd85](https://github.com/oxicrab/oxicrab/commit/fecbd851bc5dd260d7993bd937e5469261c2b18e))
- add connect timeout to pinned HTTP clients in web_fetch and http tools ([5a20815](https://github.com/oxicrab/oxicrab/commit/5a20815eb25224271db2a07bef0d4782c7c049e2))
- address 6 bugs found in codebase audit ([ac3afec](https://github.com/oxicrab/oxicrab/commit/ac3afec83487a11913bafc527261fb45d11c23fe))
- strip provider prefix from model names in routing resolution ([66bfc7b](https://github.com/oxicrab/oxicrab/commit/66bfc7bba406a9ae622d92d0f50f4b91bc4de337))
- update credential env vars count for minimax ([68797b4](https://github.com/oxicrab/oxicrab/commit/68797b46e932c11b071ceb6348d31c88e3d78883))


### Maintenance
- add .worktrees to gitignore ([ec511ce](https://github.com/oxicrab/oxicrab/commit/ec511ce3a4add2ebce14ac3eed0136c674145940))

## [0.13.2] - 2026-03-04

### Fixed
- openrouter reasoning ([7bf9085](https://github.com/oxicrab/oxicrab/commit/7bf908512e9f0389cd5922ed896556b824d38c00))
- respect per-provider temperature for tool iterations ([2ca47b6](https://github.com/oxicrab/oxicrab/commit/2ca47b6efdad5f646b89802acd01fed0927cb666))
- preserve Anthropic thinking block signature across message lifecycle ([7735ae6](https://github.com/oxicrab/oxicrab/commit/7735ae65cae25cc5ea80c1f26351f7276ef2481d))

## [0.13.1] - 2026-03-04

### Fixed
- use moonshot.ai domain instead of moonshot.cn for API base ([28589cd](https://github.com/oxicrab/oxicrab/commit/28589cdc70e1bdb814d2246dbac8779e356b93cf))

## [0.13.0] - 2026-03-04

### Added
- add tool pre-filtering by category ([a8733e6](https://github.com/oxicrab/oxicrab/commit/a8733e6093b9f952d7e77936d698f65a2508df51))
- feature-gate heavy dependencies (browser, local-whisper, embeddings) ([ebce120](https://github.com/oxicrab/oxicrab/commit/ebce120c497ff572fd265f4872f3ade9f6a9c9fa))
- **config:** add optional per-provider temperature override ([50dcf8d](https://github.com/oxicrab/oxicrab/commit/50dcf8d5391e9aa45d4d5291de2377af072ce99c))
- **cli:** add stats complexity subcommand ([a159064](https://github.com/oxicrab/oxicrab/commit/a159064b6c7b847ed30d978019dd4189283a0703))
- **db:** add get_complexity_stats() with cost correlation ([117389e](https://github.com/oxicrab/oxicrab/commit/117389ee7c4768fc4a80f6e85eef2e37af414cc1))
- **complexity:** thread request_id through agent loop for correlation ([f561e01](https://github.com/oxicrab/oxicrab/commit/f561e0182a20816f5fc545fa1b47c1334c2e5616))
- **db:** add record_complexity_event() ([ae09563](https://github.com/oxicrab/oxicrab/commit/ae09563474175b2eeafe9c64236f941a3f6c8e5a))
- **db:** add complexity_routing_log table and request_id columns ([5b1d8c1](https://github.com/oxicrab/oxicrab/commit/5b1d8c129ef9f93e7a15f857222a3201961c8caa))


### Changed
- split CLI commands into focused submodules ([74f37b8](https://github.com/oxicrab/oxicrab/commit/74f37b87c3197521bdd22f194b4f84b9a3eddf0e))
- extract all inline tests to directory module pattern ([0a391d9](https://github.com/oxicrab/oxicrab/commit/0a391d91583d1c3a9f903061b1985ff4547e7a5d))
- split agent loop module into focused submodules ([f1bb9db](https://github.com/oxicrab/oxicrab/commit/f1bb9dbc92fcb2e1f47b40261efac52485e44205))
- migrate sessions from file-based JSONL to SQLite ([76b13ff](https://github.com/oxicrab/oxicrab/commit/76b13ffb9313d143b6786c99560363fca44075d2))
- decouple MessageBus from outer Arc<Mutex<>> ([86f9169](https://github.com/oxicrab/oxicrab/commit/86f9169320e492fad229908cacf641dbf4bb8703))
- per-session processing locks for concurrent session support (A3) ([bfd214b](https://github.com/oxicrab/oxicrab/commit/bfd214b54c5e530393fdd90be68075db113a26fc))
- group AgentLoopConfig into LifecycleConfig + SafetyConfig (A6) ([04ff7de](https://github.com/oxicrab/oxicrab/commit/04ff7de3e313b763bcf84b3b192ab76bdb6c45da))
- add AgentLoopResult struct and metadata key constants (A7, A8) ([d2ef6c1](https://github.com/oxicrab/oxicrab/commit/d2ef6c194009bc256a4d90527530708cf8c9ed1c))
- inline format args, remove uninlined_format_args allow ([11104b1](https://github.com/oxicrab/oxicrab/commit/11104b1267d6f72aa0684c2cadb04ae209d7945b))
- use unwrap_or_default() for "", false, 0.0 fallbacks ([db0f79b](https://github.com/oxicrab/oxicrab/commit/db0f79baeb2e7875439fa006afc349ad4a75cd96))
- add Default to WebhookConfig, use defaults in CostGuardConfig tests ([8aa46fd](https://github.com/oxicrab/oxicrab/commit/8aa46fd40ee40c74d9e705da8a69865ed4900410))
- use CronJobState::default() and ExecutionContext defaults ([106757b](https://github.com/oxicrab/oxicrab/commit/106757bf91b02bebdfb0341aa1ca459ff97e03ba))
- add Default to InboundMessage, drop redundant fields ([f2855f6](https://github.com/oxicrab/oxicrab/commit/f2855f6e4bf4f3d0e3f25258739a45c0e48509b1))
- use ..Default::default() for ToolCapabilities construction ([d5bfbd4](https://github.com/oxicrab/oxicrab/commit/d5bfbd433ce8be55aa8727e901d187e3a6edfd49))
- add Default to OutboundMessage, drop redundant fields ([e2aa1d7](https://github.com/oxicrab/oxicrab/commit/e2aa1d78997e4a82396444a8fed904f16dd08a09))
- remove lifetime from ChatRequest, add Default ([e974afe](https://github.com/oxicrab/oxicrab/commit/e974afe248a77e2ac0091edfc9118001044f4441))
- drop redundant Default-value fields from LLMResponse sites ([aaed980](https://github.com/oxicrab/oxicrab/commit/aaed98016206aab549e16e5077b7b98015afdbca))
- use ..Default::default() for LLMResponse construction ([369f136](https://github.com/oxicrab/oxicrab/commit/369f136667a94fa7575b99de99aaf55ac933d5dc))
- **routing:** simplify model routing config from 6 concepts to 3 ([b5962f5](https://github.com/oxicrab/oxicrab/commit/b5962f55a907851a606319d899c49f6064ebe33e))
- **search:** add request_id parameter to log_search ([fd98e76](https://github.com/oxicrab/oxicrab/commit/fd98e760d45b8307c97638dde39ddd3138afe16b))
- **intent:** add request_id parameter to record_intent_event and handle_text_response ([5eb9935](https://github.com/oxicrab/oxicrab/commit/5eb993581f7753a458cd550288c3335822cae703))
- **cost:** add request_id parameter to record_cost and record_llm_call ([74b41f3](https://github.com/oxicrab/oxicrab/commit/74b41f3285099f9af30b8308c1d65f94772030fb))


### Documentation
- add allowGroups to channel config documentation ([a6eeb50](https://github.com/oxicrab/oxicrab/commit/a6eeb50a5b72a01012d099b1af32c126c6596563))
- update CLAUDE.md for M-series fixes ([a087389](https://github.com/oxicrab/oxicrab/commit/a0873897fe7f4c1c25d66e264e54380b867f7c44))
- update CLAUDE.md for architectural changes ([c2fd5e6](https://github.com/oxicrab/oxicrab/commit/c2fd5e640975336cddbb3be33b18f6f73c9d9cbd))
- **cli:** add stats complexity subcommand ([ee493e3](https://github.com/oxicrab/oxicrab/commit/ee493e36be47576ad510f430cdab7a653ce1ee06))


### Fixed
- gate markdown_code_block behind channel-telegram feature ([ff66bc8](https://github.com/oxicrab/oxicrab/commit/ff66bc8136e680a5058161608d427026766190a7))
- session cleanup with TTL 0 deletes all sessions ([3ea5ba8](https://github.com/oxicrab/oxicrab/commit/3ea5ba8e0479e5d5742731a3baad6b52a89a72e0))
- resolve clippy warnings from architectural changes ([615124a](https://github.com/oxicrab/oxicrab/commit/615124ad9908ad247ceb7ce435f3840edb02aa42))
- low-severity findings from code review (L1-L5,L7,L8,L11-L14) ([8b9725c](https://github.com/oxicrab/oxicrab/commit/8b9725cf6eab4d0ffead70b1965f655e5dbf52ba))
- group access control and webhook replay protection (M6, M11) ([5114ca2](https://github.com/oxicrab/oxicrab/commit/5114ca2bb7c1f626699d92302abfa7dd296016af))
- browser SSRF check after eval/click/navigate actions (M4) ([61e63cf](https://github.com/oxicrab/oxicrab/commit/61e63cfb1e1f23debff8e6ffde82206de410d921))
- reconstruct tool_calls and tool_call_id from session history (M14) ([3999820](https://github.com/oxicrab/oxicrab/commit/3999820d770c7e5048b4be3f823604bd050ff968))
- remaining medium-severity findings (M2, M9) ([1e3521b](https://github.com/oxicrab/oxicrab/commit/1e3521b16157b2414f2758a69a5e6c1590cde53c))
- medium-severity findings from code review (M1,M3,M5,M7,M8,M10,M12,M13,M15) ([5775a4a](https://github.com/oxicrab/oxicrab/commit/5775a4ae7332222f3710ccb92ca0c018f4dfb5b5))
- three high-priority bugs from code review ([6d415e2](https://github.com/oxicrab/oxicrab/commit/6d415e25b549d69323c8baadda675c410a02c062))
- **cost:** attribute fallback provider costs to actual serving model ([b9e5e71](https://github.com/oxicrab/oxicrab/commit/b9e5e710ce534ddd5d38a2c1bd28e240ce794304))
- **deps:** update aws-lc-sys to 0.38.0 for PKCS7_verify bypass fix ([6a85982](https://github.com/oxicrab/oxicrab/commit/6a8598299bd87bce410a52b1efc6898fda83894c))
- **routing:** support "default" tier for complexity routing fallback to default model ([ae31bd3](https://github.com/oxicrab/oxicrab/commit/ae31bd37c257c21d48c05d207ff91a82b4e484af))


### Testing
- add end-to-end bus pipeline integration test (A5) ([7ee586b](https://github.com/oxicrab/oxicrab/commit/7ee586bf817bf6904aeac88a6925479b01bf93d9))

## [0.12.0] - 2026-03-03

### Added
- **routing:** add complexity-aware message routing ([0efb87b](https://github.com/oxicrab/oxicrab/commit/0efb87be59c74ef75c073288ce72547779c00d46))
- **gateway:** wire ResponseFormat through HTTP API to LLM providers ([b8f0bfd](https://github.com/oxicrab/oxicrab/commit/b8f0bfd5ad73f485f7d7a8f83c7457f28fd991e4))
- **gateway:** add per-IP rate limiting via governor ([7225188](https://github.com/oxicrab/oxicrab/commit/722518837d6704631d4333ebe426e5545599674b))
- **routing:** build fallback chain from modelRouting.fallbacks config ([65ae5f0](https://github.com/oxicrab/oxicrab/commit/65ae5f0b13ee073fdcb07d1d001e8dea57d6d624))
- **routing:** wire model routing into daemon, cron, subagent, compaction ([79ba744](https://github.com/oxicrab/oxicrab/commit/79ba744a18e7dfadd0e704c814d481bbee2e9273))
- **routing:** add ModelRoutingConfig, ResolvedRouting, and provider override ([cdfd0d4](https://github.com/oxicrab/oxicrab/commit/cdfd0d4548df4d2bf2698fb9d74d1287ebe7cad5))
- **providers:** extend FallbackProvider to Vec-based chain ([448e3cf](https://github.com/oxicrab/oxicrab/commit/448e3cfccbeab630e9dab7b4d6f1b5742174980e))
- **deploy:** add docker-compose.yml with health check and volume mount ([3118d10](https://github.com/oxicrab/oxicrab/commit/3118d10ae402c8308b169ca7e818cc9737f421bc))
- **deploy:** add HTTP health check script ([1e45e22](https://github.com/oxicrab/oxicrab/commit/1e45e22ec6143a208a8a9474ee116ead31582ba2))
- **gateway:** add API key authentication for chat and A2A endpoints ([9d5ae72](https://github.com/oxicrab/oxicrab/commit/9d5ae72c385a74724cb03dd4111fca61e5807486))
- **leaks:** Incoming leak detection ([041af6a](https://github.com/oxicrab/oxicrab/commit/041af6aba1dc5c19ba1df1b865be000a77c8931a))


### CI/CD
- optimize clippy and package-linux cache usage ([08b0ef2](https://github.com/oxicrab/oxicrab/commit/08b0ef2ec411314c63a0ae8c6aa1a7fe2e4a01a5))


### Changed
- **loop:** extract hallucination and compaction_history submodules ([5b113fa](https://github.com/oxicrab/oxicrab/commit/5b113fa605a272cb3db2f0b07b16e7ba1981441c))
- **memory_db:** extract stats, embeddings, search, indexing submodules ([8c2ecdc](https://github.com/oxicrab/oxicrab/commit/8c2ecdc8c2687f599417b2f67ea279776e7afe73))
- **memory_db:** extract cost, dlq, workspace into submodules ([0a3ee26](https://github.com/oxicrab/oxicrab/commit/0a3ee26c4734a4b50e2a7593f89a6e8918b31382))
- **gateway:** extract magic numbers to named constants ([b0516c6](https://github.com/oxicrab/oxicrab/commit/b0516c6609f961f239a3cec37299710a679b87ea))
- Update src/cron/service/mod.rs ([506cc89](https://github.com/oxicrab/oxicrab/commit/506cc894d2512af5e3f6b68a9f2b0691166173d1))
- **config:** remove localModel field in favor of modelRouting.fallbacks ([c98a39b](https://github.com/oxicrab/oxicrab/commit/c98a39b640df2ca6d8ea362e352d8cf076bea25a))
- **config:** remove provider field and --provider CLI flag in favor of prefix notation ([f91230f](https://github.com/oxicrab/oxicrab/commit/f91230ffcedffc317bbb090c9f32b6bc29600d65))
- **config:** move promptGuidedTools to LocalProviderConfig for ollama/vllm only ([0b5d693](https://github.com/oxicrab/oxicrab/commit/0b5d6937275342eabb48d04e999a030bb25a6696))


### Documentation
- rewrite model routing section with narrative explanation ([c421b05](https://github.com/oxicrab/oxicrab/commit/c421b0512c7dea389f71cb1bdf42c48b951a86be))
- mention complexity routing in README model routing summary ([a610e5c](https://github.com/oxicrab/oxicrab/commit/a610e5cce582c8368ff1b937b4eb519a4003cc6b))
- fix stale paths, counts, references, and missing fields across all docs ([77fcebd](https://github.com/oxicrab/oxicrab/commit/77fcebd23672a9773ee545724d0fa50eca388dd6))
- clarify model routing scope, add resolution diagram, update README ([0dee19a](https://github.com/oxicrab/oxicrab/commit/0dee19a18a0b411f1ad8e86b9be0e952de709fc2))
- add model routing, fallback chain, and rate limiting documentation ([26ce995](https://github.com/oxicrab/oxicrab/commit/26ce995b79b2bfa132273c1d54ad6b32c57c9034))
- add VPS deployment guide with Tailscale and dual-VPS monitoring ([efb3411](https://github.com/oxicrab/oxicrab/commit/efb34114ddc879e8b8981572f812cdbf6a92138d))
- update ARCHITECTURE.md for apiBase/headers and LocalProviderConfig changes ([87b6c73](https://github.com/oxicrab/oxicrab/commit/87b6c73e3e08b203ea27af3f8d45bbff16d34dfd))
- add design for fixing dead config fields ([51135fa](https://github.com/oxicrab/oxicrab/commit/51135fa73fd26224509e9b366e46c20a49c46b78))
- update CLAUDE.md and ARCHITECTURE.md for audit fixes ([137342f](https://github.com/oxicrab/oxicrab/commit/137342f76d72b197f5d20cee54d15a5958e9da56))
- update claude and readme ([d07c5e5](https://github.com/oxicrab/oxicrab/commit/d07c5e5e822e315f5b39be168ea76992d6d3e54f))


### Fixed
- **clippy:** resolve collapsible-if and map-unwrap-or warnings ([ce636f5](https://github.com/oxicrab/oxicrab/commit/ce636f515fa3de534e86080a64e0d6f4cfd27dbd))
- **sandbox:** log warning when workspace path canonicalization fails ([99a1a56](https://github.com/oxicrab/oxicrab/commit/99a1a56003bf369974d88ac8e5dd551ff5d0b9e3))
- **gateway:** log warning on mutex poison recovery instead of silently continuing ([4fd4a58](https://github.com/oxicrab/oxicrab/commit/4fd4a58c5ea19506380d98174e9f29d785601619))
- **security:** add 5s DNS resolution timeout to prevent DoS via slow nameservers ([da95d7a](https://github.com/oxicrab/oxicrab/commit/da95d7af916891808f71b702fd202f9a707983a1))
- **cron:** add missing brace and split chained if-let in tick loop ([e2284ad](https://github.com/oxicrab/oxicrab/commit/e2284ad4d5c84213a2381d97bd3225a2f690a712))
- fix: Fixed the CodeQL security alert about uncontrolled allocation size in ([e4c4236](https://github.com/oxicrab/oxicrab/commit/e4c4236008a38a0e0e84a795d90a178125c56b11))
- address 15 findings from full codebase security and correctness review ([fb9ccf0](https://github.com/oxicrab/oxicrab/commit/fb9ccf09bc922702a51dcb81d80ff60f22463e2d))
- **routing:** harden complexity scorer against invalid config and silent failures ([84dfa5b](https://github.com/oxicrab/oxicrab/commit/84dfa5bc52ac2c412d51e1147000a06897a63e67))
- **routing:** address code review findings in complexity scorer ([ced7a7e](https://github.com/oxicrab/oxicrab/commit/ced7a7e192103ae7c97f35dfa5cab2d4b4724221))
- **routing:** improve daemon model logging, warn on localModel+fallbacks conflict ([0223538](https://github.com/oxicrab/oxicrab/commit/0223538885eba47a247b6df62bd09a72935b3c65))
- **gateway:** use socket addr for rate limiting, exempt health, dynamic Retry-After ([2c08a04](https://github.com/oxicrab/oxicrab/commit/2c08a04d6976d6869962f0ed62a0c4ada91ab415))
- **routing:** correct prompt-guided tools wrapping for routed and fallback providers ([4fdc972](https://github.com/oxicrab/oxicrab/commit/4fdc9726884ca15771e5b8dd0075aa8ca6884c2a))
- **providers:** add OpenRouter multi-slash test, remove dead infer branch ([745da93](https://github.com/oxicrab/oxicrab/commit/745da93f47879611926f8d0c141e2405845d4c15))
- **deploy:** use HTTP health check, fix exposed port to 18790 ([2bb7d7a](https://github.com/oxicrab/oxicrab/commit/2bb7d7a5d9d675ff2e3cd5a2c76a6c5139938c5e))
- **config:** remove promptGuidedTools from non-local provider configs ([9618191](https://github.com/oxicrab/oxicrab/commit/961819102b00631a92f6f7724e1d316a09d2ab59))
- **config:** remove dead executionProvider field from DaemonConfig ([c191d95](https://github.com/oxicrab/oxicrab/commit/c191d951abdcc8830e9c5b4602de7a628f08b4d2))
- **providers:** wire up apiBase and headers for OpenAI and Gemini providers ([4cced63](https://github.com/oxicrab/oxicrab/commit/4cced6323468d48d1d4d14edc4b8f7934afbc166))
- **providers:** wire up apiBase and headers for Anthropic provider ([292563c](https://github.com/oxicrab/oxicrab/commit/292563c22a666b45cd52e049d4a26c77ea323438))
- **obsidian:** use path component check instead of substring for traversal detection ([72c1db1](https://github.com/oxicrab/oxicrab/commit/72c1db187081f385604ccf85fe7677832c2a2740))
- address round-2 audit findings (26 issues) ([b232e6f](https://github.com/oxicrab/oxicrab/commit/b232e6f551b978e5298885188d52b802bc804428))
- harden obsidian path traversal and simplify event matcher init ([0e62879](https://github.com/oxicrab/oxicrab/commit/0e62879e2cd6464cace9bb03f5e7c39200999775))
- address remaining audit issues (channels, CLI, tools) ([56c9007](https://github.com/oxicrab/oxicrab/commit/56c90074704f263604aaf19f59f2b87e145e57c7))
- address 43 issues from full codebase audit ([4d101bc](https://github.com/oxicrab/oxicrab/commit/4d101bc7e3c6f1e40179c857d89c2f90f1db4ed4))
- This keeps the original &str when no redaction is needed, avoiding the allocation. ([52ddd4b](https://github.com/oxicrab/oxicrab/commit/52ddd4b5a3d09752d91854c957fc66e18c4381dd))


### Removed
- Removed design docs ([d81a317](https://github.com/oxicrab/oxicrab/commit/d81a3172a2bb9929aaa45d30ac6ad1dbfc13e5e8))

## [0.11.7] - 2026-02-28

### Added
- **channels:** log successful media uploads across all channels ([0e4287e](https://github.com/oxicrab/oxicrab/commit/0e4287e62484ff916a5ec157e1759b1ccad6330c))
- **workspace:** add send action to workspace tool for file delivery ([d87a1d6](https://github.com/oxicrab/oxicrab/commit/d87a1d6168f39fdcf6c1b0235bf9689e44000ad8))
- **workspace:** integrate workspace file cleanup into hygiene cycle ([af8ee0a](https://github.com/oxicrab/oxicrab/commit/af8ee0ae4df942ee73456eb26fd5e71a0b75d306))
- **workspace:** integrate WriteFileTool and ReadFileTool with workspace manifest ([7e9f1a0](https://github.com/oxicrab/oxicrab/commit/7e9f1a0e53cb16de23975cf574513803d1dd675b))
- **workspace:** wire WorkspaceManager through ToolBuildContext and register workspace tool ([12a53b2](https://github.com/oxicrab/oxicrab/commit/12a53b2a95bd57e751a3cf74ad150f829a7d8484))
- **workspace:** add workspace action-based tool with 8 actions ([f06d083](https://github.com/oxicrab/oxicrab/commit/f06d083bbe34f6d5eb6d7e9765ea5ed31e886d22))
- **workspace:** add WorkspaceTtlConfig to agent defaults ([a3216fa](https://github.com/oxicrab/oxicrab/commit/a3216fa5a79c7103b2e196ee4dfd67101f9a2364))
- **workspace:** add manifest integration methods to WorkspaceManager ([32b6777](https://github.com/oxicrab/oxicrab/commit/32b67771fd03663633250411409531d36366c382))
- **workspace:** add workspace_files manifest table and CRUD methods to MemoryDB ([451e8b7](https://github.com/oxicrab/oxicrab/commit/451e8b7d35c400717c0ed2c1bfe9f8386de1eea9))
- **workspace:** add WorkspaceManager with category inference and path resolution ([90221d1](https://github.com/oxicrab/oxicrab/commit/90221d1f55f670776e666f7c3580d244e469ab36))


### CI/CD
- bump actions/download-artifact from 7 to 8 ([e9658c1](https://github.com/oxicrab/oxicrab/commit/e9658c1fa9506a89bbe032b6c961cf1634eacc1d))
- bump actions/upload-artifact from 6 to 7 ([19468b3](https://github.com/oxicrab/oxicrab/commit/19468b3b4e0a02ec46103701c340f6f7066bbfc3))


### Dependencies
- bump rmcp from 0.16.0 to 0.17.0 ([dbefc53](https://github.com/oxicrab/oxicrab/commit/dbefc53f9e2629acbae7028d7c19630c5bcbb70d))
- bump chrono from 0.4.43 to 0.4.44 ([944f359](https://github.com/oxicrab/oxicrab/commit/944f3593201883b05606cb0b8dfec1956d824995))


### Documentation
- **plans:** Removed design docs for tool metadata ([5dcb27a](https://github.com/oxicrab/oxicrab/commit/5dcb27a363221a4cc6c7758aea4595307dc2de7a))
- add workspace manager notes to CLAUDE.md ([b1a2fd0](https://github.com/oxicrab/oxicrab/commit/b1a2fd052d0ca0fc64989c1b9f5ae8ae3fadb08d))
- add workspace tool and workspaceTtl config documentation ([abb4774](https://github.com/oxicrab/oxicrab/commit/abb47748e1fdd9066efccd20549c089002d2e5f2))
- add workspace manager implementation plan ([786784b](https://github.com/oxicrab/oxicrab/commit/786784b7d0745d10dbb92c8bac02f2e1a822c98f))
- add workspace manager design document ([ceb2f26](https://github.com/oxicrab/oxicrab/commit/ceb2f260cb009ba21f8a8fb52807f477bc7650ed))


### Fixed
- **workspace:** allow sending any workspace file, not just managed categories ([118e5dd](https://github.com/oxicrab/oxicrab/commit/118e5dd8a60cd19f07ba76967c9453ae9224b86b))
- **cron:** enforce 60-second minimum for every_seconds interval ([a6eb6ca](https://github.com/oxicrab/oxicrab/commit/a6eb6ca2c8f4b6c3e4a328fa37a8b156b6d86682))
- **workspace:** use canonical workspace_root in traversal test assertion ([646a464](https://github.com/oxicrab/oxicrab/commit/646a4646d5056ea66f91a83fb8a533da250ebd04))
- **workspace:** canonicalize input paths for macOS symlink compatibility ([e9cf0d6](https://github.com/oxicrab/oxicrab/commit/e9cf0d6a1dab82df4d6c8556399c9592a7de0737))
- **workspace:** canonicalize workspace root for consistent path matching ([dc86f73](https://github.com/oxicrab/oxicrab/commit/dc86f739aa7ea7d484e326ad94ed340cf16166ee))
- **workspace:** add path validation guards and improve cleanup clarity ([932b8af](https://github.com/oxicrab/oxicrab/commit/932b8afb0a3daa72c390359f465d0b92f0752a14))
- **workspace:** use ON CONFLICT upsert and fix tag search false positives ([7a2cebf](https://github.com/oxicrab/oxicrab/commit/7a2cebfa1f5bf8cdb82e71a47913934fe00efd07))
- **workspace:** sanitize path traversal and add Display/Serialize to FileCategory ([cc37709](https://github.com/oxicrab/oxicrab/commit/cc3770967539b5a10cd123fa8e70409561b28be7))


### Testing
- add workspace management integration tests ([b349d60](https://github.com/oxicrab/oxicrab/commit/b349d600845c3aa62810bcf56badd301b4c479a4))

## [0.11.6] - 2026-02-26

### Changed
- **cron:** use humantime for duration formatting ([ac07187](https://github.com/oxicrab/oxicrab/commit/ac07187bd13601980bff0fba2a341b17baac24fa))
- replace hand-rolled floor_char_boundary with stdlib ([ab1ee95](https://github.com/oxicrab/oxicrab/commit/ab1ee95f84b468046ad8209f87a57f4f3672d43f))
- extract truncate_chars utility; consolidate GitHub API methods ([ec7962b](https://github.com/oxicrab/oxicrab/commit/ec7962b898e7a3b9b44a5b88a3759477c18906ee))
- **tools:** reduce boilerplate with actions! macro and helpers ([1e55ca1](https://github.com/oxicrab/oxicrab/commit/1e55ca16181ffee9a0016dd3f6736752acc0cd3b))
- extract inline tests to directory modules (13 files) ([fe8758c](https://github.com/oxicrab/oxicrab/commit/fe8758ca5dcad54e6ee704c0bdad7b2290ca0238))
- **agent-loop:** extract ToolConfigs from AgentLoopConfig ([02cf2cd](https://github.com/oxicrab/oxicrab/commit/02cf2cdb3c7cc293796f25612833e28cdd6759e8))
- **agent-loop:** replace correction bool with CorrectionState struct ([812f6ad](https://github.com/oxicrab/oxicrab/commit/812f6ad71af31f07c043be7054adc3912318cd5e))


### Fixed
- **cron:** avoid deadlock when run/dlq_replay called from agent turn ([a76017a](https://github.com/oxicrab/oxicrab/commit/a76017a6fe3052a239bb537c154d1d732c315b4e))
- **cron:** O(1) interval catch-up; fix session lock coordination ([8005c04](https://github.com/oxicrab/oxicrab/commit/8005c047bf0797dda9a59c62664bae43c3ffc82e))
- address 3 issues from config and utils deep review ([8b0e5ef](https://github.com/oxicrab/oxicrab/commit/8b0e5ef9d48950d6d420f9120047ca45a75614b8))
- **tools:** address 4 issues from deep review ([645460e](https://github.com/oxicrab/oxicrab/commit/645460ebf366ad72ff93614c01e30f83031f19f8))
- **gateway:** address 4 issues from deep review ([c9c822c](https://github.com/oxicrab/oxicrab/commit/c9c822c16ae322d8a3445f1e36a780f8ada76291))
- **channels:** address 7 issues from deep review ([83a8ffc](https://github.com/oxicrab/oxicrab/commit/83a8ffc56a7b7ed7e75cb47be5cff6235226db86))
- **safety:** address 5 issues from deep review ([6f12cde](https://github.com/oxicrab/oxicrab/commit/6f12cdedd8756a32e31aeda6f4c1fd77c0940072))
- **providers:** address 8 issues from deep review ([54219c5](https://github.com/oxicrab/oxicrab/commit/54219c5ca2307b6aeb3df599fbce63115f2e4d00))
- **memory-db:** address 8 issues from deep review ([1d42cff](https://github.com/oxicrab/oxicrab/commit/1d42cffb2d61e9d8173e735701974c98aac4f7a6))
- **compaction:** address 7 issues from deep review ([8863ca0](https://github.com/oxicrab/oxicrab/commit/8863ca0f8284d9ff966f55bcec730a42dc7b2e44))
- **agent-loop:** address seven review findings across the agent loop ([fb86f08](https://github.com/oxicrab/oxicrab/commit/fb86f08554bea3c67e38f0912a4f95e0f532f31a))
- **agent-loop:** five hallucination detection improvements ([8eb7304](https://github.com/oxicrab/oxicrab/commit/8eb7304c3cf190ce04475bb08f703135e4eb0b5a))
- **agent-loop:** prevent infinite hallucination correction loop ([f4fd204](https://github.com/oxicrab/oxicrab/commit/f4fd204968520814e46c3f2c292503b3deddaca8))
- **weather:** change subagent access from ReadOnly to Full ([3266174](https://github.com/oxicrab/oxicrab/commit/3266174dde1fa9926cc402262e33372a7957984a))
- **subagent:** populate prompt with tool metadata to prevent exec misuse ([0846fb8](https://github.com/oxicrab/oxicrab/commit/0846fb8ee33eaf7464b6a3a21c4f782d1e6d950e))


### Maintenance
- remove dead code (LOGO constant, unused cache_len test helper) ([512fe87](https://github.com/oxicrab/oxicrab/commit/512fe876f9aba9ea97a7fd26a58a7968ea97eec5))

## [0.11.5] - 2026-02-25

### Added
- **cli:** add shell completion command ([3603510](https://github.com/oxicrab/oxicrab/commit/360351009cd2bcf3c71e38d51137396689ef0182))
- **cli:** add --version flag ([c4880d8](https://github.com/oxicrab/oxicrab/commit/c4880d87365410ee43169d77b01a71e562642a11))
- **mcp:** use built_in capability for shadow protection ([c34d668](https://github.com/oxicrab/oxicrab/commit/c34d668758752a49eebdb72bcb940014c46cf6e6))
- **exfil:** replace blocked_tools list with network_outbound capability ([fc67ca1](https://github.com/oxicrab/oxicrab/commit/fc67ca12175eea3348b56bf420b30377c32aa31a))
- **subagent:** build subagent tools from capabilities instead of hardcoded list ([34881ae](https://github.com/oxicrab/oxicrab/commit/34881ae1248fb7277f69b33ea5728b23d7ff7109))
- **tools:** add ReadOnlyToolWrapper with dual enforcement ([9e97f18](https://github.com/oxicrab/oxicrab/commit/9e97f18972167d606b36c8385c61a2c56c5bb0b8))
- **tools:** annotate action-based tools with per-action capabilities ([43295d5](https://github.com/oxicrab/oxicrab/commit/43295d5bb54582fec2223ae1ff8ea102f81bf36b))
- **tools:** annotate single-purpose tools with capability metadata ([e427336](https://github.com/oxicrab/oxicrab/commit/e42733682ded2a0f2c782251f567eee46932f665))
- **tools:** add ToolCapabilities types and capabilities() trait method ([add7d8a](https://github.com/oxicrab/oxicrab/commit/add7d8ada36a3954b2df5487718a49c08be0e72c))
- **subagent:** add per-subagent activity log and tool visibility ([e98bbdd](https://github.com/oxicrab/oxicrab/commit/e98bbddec2a45fef14d76d8da29eb276cd19c013))


### Documentation
- document subagent tool access and capability metadata ([3eda2d3](https://github.com/oxicrab/oxicrab/commit/3eda2d37e66cb3be34ca7df4e167a746889fb844))
- **plans:** Removed design docs for tool metadata ([cbb1cf5](https://github.com/oxicrab/oxicrab/commit/cbb1cf538de8b21ffe18474ea52b208d10f2bc2c))
- update docs for capability-based tool metadata ([9ceaba3](https://github.com/oxicrab/oxicrab/commit/9ceaba3c7cf391dfa186193aa0503253de02edb3))
- **plans:** add tool capability metadata implementation plan ([fee1a3b](https://github.com/oxicrab/oxicrab/commit/fee1a3b46d87092e0a76f0a6f4c92ba91c61921a))
- **plans:** add tool capability metadata design ([69508aa](https://github.com/oxicrab/oxicrab/commit/69508aaa41f9854293078d3ed4544859c2e4a983))


### Fixed
- **exec:** make command parsing quote-aware to prevent false rejections ([7042903](https://github.com/oxicrab/oxicrab/commit/704290347f3cbd04a9fd858ae31f435c9039a3e2))

## [0.11.4] - 2026-02-25

### Added
- **agent:** add intent metrics recording and stats CLI subcommand ([5df0a62](https://github.com/oxicrab/oxicrab/commit/5df0a628628979470e2466bc0efcf99deb79353c))
- **agent:** add intent-based hallucination detection with semantic embedding fallback ([ba84433](https://github.com/oxicrab/oxicrab/commit/ba8443308afed271ab0a27933a775b7d51bea44d))
- tweaked prompt ([ceb50f8](https://github.com/oxicrab/oxicrab/commit/ceb50f823c1b40b822141d97144b740ea7e4940a))
- discourse fixes ([50815af](https://github.com/oxicrab/oxicrab/commit/50815af3717f790e291dfd55beb307508ed7f9c4))
- **memory:** add quality gates and negative memory reframing ([be253dc](https://github.com/oxicrab/oxicrab/commit/be253dc792a9e3898b5a986c03529f18e715b9f6))
- **truncation:** add tool result blob sanitization ([e6d6a47](https://github.com/oxicrab/oxicrab/commit/e6d6a4798e8396f3d4775180c9aba61f44771fe8))
- **memory:** add recency-weighted BM25 search scoring ([8487119](https://github.com/oxicrab/oxicrab/commit/848711905f3120ec969406c9719026522b307435))
- **compaction:** add orphan tool message cleanup post-compaction ([05f60cf](https://github.com/oxicrab/oxicrab/commit/05f60cfa104c38a08f35d0978761c11148116558))
- **safety:** use Aho-Corasick for two-phase leak detection ([9bd8d34](https://github.com/oxicrab/oxicrab/commit/9bd8d34c60b2da6572b5033e5724732fa7c80104))
- **context:** add plugin context providers ([9d378a4](https://github.com/oxicrab/oxicrab/commit/9d378a45c070ecf6b4c24e42110d7b6e08a946f6))
- **compaction:** add pre-compaction memory flush ([8f53d77](https://github.com/oxicrab/oxicrab/commit/8f53d77fc8a78a8f89f793c3c41340973de881e3))
- **memory:** add explicit "remember" fast path ([5974c81](https://github.com/oxicrab/oxicrab/commit/5974c81d8ca529e50e6d15f725018f35f67ff6fa))
- **cron:** add dead letter queue for failed job executions ([b715875](https://github.com/oxicrab/oxicrab/commit/b71587594cc37d53f51c58817f8dfa7a650f19e3))
- **fuzz:** add cargo-fuzz targets for security-critical parsers ([05640a7](https://github.com/oxicrab/oxicrab/commit/05640a72291c0e67432e379ce40987bc86636666))
- **gateway:** add --echo mode for LLM-free channel testing ([0a0e756](https://github.com/oxicrab/oxicrab/commit/0a0e7562785d293e03d0cb7d0263d9bf0d0f86a5))
- **gateway:** add A2A protocol support ([cb991be](https://github.com/oxicrab/oxicrab/commit/cb991bef6374750d790583dd7351f68254aa7368))
- **gateway:** add enabled flags and expand test coverage ([94e1808](https://github.com/oxicrab/oxicrab/commit/94e180843f03d614b41280f3be1949387eebf7df))
- **memory:** add knowledge directory for RAG document ingestion ([df58ce3](https://github.com/oxicrab/oxicrab/commit/df58ce3d8c3155a41e0d17cf51af028cc5da14f2))
- **gateway:** add generic webhook receiver endpoint ([e98074a](https://github.com/oxicrab/oxicrab/commit/e98074a02485cef56e6580278b365670681def12))
- **gateway:** add HTTP API server with POST /api/chat and GET /api/health ([457e075](https://github.com/oxicrab/oxicrab/commit/457e0759966069bf7a7290540b38781ae166bddc))
- **providers:** add PDF document support across all providers ([8aed9be](https://github.com/oxicrab/oxicrab/commit/8aed9be0bd036844b4888caa00d87696de4dc873))
- **providers:** add JSON mode and structured output support ([5f62f49](https://github.com/oxicrab/oxicrab/commit/5f62f49d90346da795307143927bec5e084f9911))
- **memory:** add LRU cache for query embeddings ([701702f](https://github.com/oxicrab/oxicrab/commit/701702f44616673995fab7a2e2be319fddf2d93e))
- **memory:** add configurable hybrid search fusion strategy ([02f072b](https://github.com/oxicrab/oxicrab/commit/02f072b0e06f95bb05d4aede6f98ba6a1d9fdece))
- **memory:** isolate personal memory from group chats ([9233298](https://github.com/oxicrab/oxicrab/commit/9233298fac8ac2fb191bfe0d72e8ebb34a7f0755))
- **cron:** propagate origin metadata through cron job lifecycle ([23d5da2](https://github.com/oxicrab/oxicrab/commit/23d5da25e84d14f98cc7ac3b6f29dc251adeb942))


### CI/CD
- add [no ci] commit message flag to skip all CI jobs ([3dc3b4a](https://github.com/oxicrab/oxicrab/commit/3dc3b4a509a8fd78b1f703481ff89379a96ab595))
- make package-linux self-contained like package-macos ([e81abef](https://github.com/oxicrab/oxicrab/commit/e81abef82ce16451b211dbade3de7cfe29c0a54d))
- add config auto-generation test, docs freshness checks, and CI path filtering ([ba8f980](https://github.com/oxicrab/oxicrab/commit/ba8f980b58c510f7beaa4e1530422fe1abc6f23e))


### Changed
- extract inline tests to separate files for 20 modules ([6a27e68](https://github.com/oxicrab/oxicrab/commit/6a27e680c3ff9bfca57eaf0a980f7133decb061a))


### Documentation
- update config example, CLI, and docs for recent features ([c773d97](https://github.com/oxicrab/oxicrab/commit/c773d9766a203c1e10e481db882464cfec6efe5f))
- **claude:** document 7 recent features in CLAUDE.md ([971d772](https://github.com/oxicrab/oxicrab/commit/971d772f2d05694f8ef1f78484c56f6ef890b526))
- add cross-review of 6 competitor repos (Feb 23) ([127e147](https://github.com/oxicrab/oxicrab/commit/127e147c3732eac3493ec03750eb4fecf730f773))
- update CLAUDE.md for test split threshold, gateway flags, and router testing ([5c0752a](https://github.com/oxicrab/oxicrab/commit/5c0752aa20ec84a401fd6c4f0c54af6a38e738d4))
- update docs for gateway API, webhooks, knowledge dir, and media ([a4a3bde](https://github.com/oxicrab/oxicrab/commit/a4a3bdea4d9462fdb03541236fec10c4c8bf04b4))
- rewrite README.md as concise summary with docs links ([9588367](https://github.com/oxicrab/oxicrab/commit/958836779a13f2dba6136ab8b7e11e61358305cd))
- document resource limits, tool constraints, and hardening patterns ([dab13f5](https://github.com/oxicrab/oxicrab/commit/dab13f5c29c3a9a8e1c6f7fa2c7ba475804ae831))
- update CLAUDE.md and ARCHITECTURE.md for recent features ([98e556a](https://github.com/oxicrab/oxicrab/commit/98e556acb19575bb3f14a47992774162bf195aff))
- document prompt caching, cron metadata, and known gaps ([b31a882](https://github.com/oxicrab/oxicrab/commit/b31a8820343dfb10d191461195ca44fdf4494421))


### Fixed
- **providers:** model prefix overrides explicit provider setting ([3ed6469](https://github.com/oxicrab/oxicrab/commit/3ed64698ef2a4f222c22e006cd049069fd342294))
- **oauth:** add 401 retry to warmup and log provider selection ([c34e6e8](https://github.com/oxicrab/oxicrab/commit/c34e6e84ec2775eae825a1e6a6856a2500ee9d82))
- **agent:** deduplicate fact extraction and use section-based daily notes ([ef263ff](https://github.com/oxicrab/oxicrab/commit/ef263ffc192ed18da7b1eaf3801b07a2e9e55036))
- **agent:** move tool facts from user message to system prompt ([cb99eaf](https://github.com/oxicrab/oxicrab/commit/cb99eaf9c83bbfd86dd3deb5b0f3a4d9f0920060))
- **agent:** fix empty response, orphan stripping, event matcher race, session save race ([9eacdc5](https://github.com/oxicrab/oxicrab/commit/9eacdc5bef66043302734ba7300f68327238ad47))
- **agent:** remove tool_choice="any" forcing and fix conversation flow ([450a811](https://github.com/oxicrab/oxicrab/commit/450a811e12b9f68f516ec1ed45b7e1b5d7b7f03d))
- **memory:** preserve leading whitespace when reframing lines ([9786c80](https://github.com/oxicrab/oxicrab/commit/9786c80324067b65f8a109096d11a86d0890ea21))
- **safety:** use empty prefix for Discord token instead of "." ([d236d40](https://github.com/oxicrab/oxicrab/commit/d236d4061a20a5a327b666b7b9689b775008754e))
- **context:** include stderr in provider output ([db92da4](https://github.com/oxicrab/oxicrab/commit/db92da4d36764e290ffee03f0e4a3b1400fdf6fd))
- **memory:** add parentheses for operator precedence clarity ([1ebfebe](https://github.com/oxicrab/oxicrab/commit/1ebfebe6b1b00a4574c6b2adc53b12aab33f5b39))
- **cron:** increment retry_count on DLQ replay ([f58af4a](https://github.com/oxicrab/oxicrab/commit/f58af4a243e0f27fe9fb9780838b6c2d5c65b057))
- **a2a:** remove unwrap on task serialization ([fc0ed60](https://github.com/oxicrab/oxicrab/commit/fc0ed60f14c43da0f5cd720d919f01027b54784e))
- **gateway:** add a2a_config param to echo-mode gateway start ([fcb8adf](https://github.com/oxicrab/oxicrab/commit/fcb8adf7a81009f635411dfca52dbcd3ed913740))
- **whatsapp:** handle document and video media downloads ([6dd5904](https://github.com/oxicrab/oxicrab/commit/6dd590407e6999deadaddc7056e55b2370d936cb))
- **providers:** preserve reasoning_content across message lifecycle ([536139e](https://github.com/oxicrab/oxicrab/commit/536139e48292f00ae0603b788d76106db45bffe2))


### Maintenance
- remove stale cross-review workspace file ([95dc6e4](https://github.com/oxicrab/oxicrab/commit/95dc6e405fe38eed083a5b7af8173414fca714be))
- merge main into feat/a2a-protocol ([e6768dd](https://github.com/oxicrab/oxicrab/commit/e6768dd916face61cdcf6c7d0295b46b322722d6))
- **readme:** updated readme with motives ([f4e4cae](https://github.com/oxicrab/oxicrab/commit/f4e4cae2adb21e1776125ad3cefca2528e809be5))
- add .fastembed_cache to gitignore ([aceba2f](https://github.com/oxicrab/oxicrab/commit/aceba2f6f1438b3f3ff6ea0b76be343dfcbf01e0))
- **crates:** Updated crates ([f95caab](https://github.com/oxicrab/oxicrab/commit/f95caab5af5c7df83d9da5b6ac73c267d6c34910))


### Performance
- parallelize startup for faster boot time ([1046e84](https://github.com/oxicrab/oxicrab/commit/1046e84181cb532f542b76685737924a26331efb))


### Testing
- add 25 unit tests across 4 untested modules ([b2ca75d](https://github.com/oxicrab/oxicrab/commit/b2ca75de14c0f16d902fdc67d6dce8a2df18ce63))
- add 40 tests for recent features ([2ddce3c](https://github.com/oxicrab/oxicrab/commit/2ddce3c78da9c880dbda40482ac67282906ecf8e))
- add 45 unit tests across config, gateway, openai, and transcription ([4ad4fa6](https://github.com/oxicrab/oxicrab/commit/4ad4fa6c9cfd4bc35a6a3f3aeb06c22ec62d83ad))

## [0.11.3] - 2026-02-22

### Added
- **memory:** add search tracking, cost persistence, hybrid search, and CLI stats ([3d36c6d](https://github.com/oxicrab/oxicrab/commit/3d36c6dab49d897907844a48a0ca3d93e7337817))


### Changed
- extract inline tests to separate files ([860f5ef](https://github.com/oxicrab/oxicrab/commit/860f5ef766e793ad15f456acec1cde4c8c54d56e))


### Fixed
- **test:** fixed test assertion and the actual error message ([eda4e9f](https://github.com/oxicrab/oxicrab/commit/eda4e9fc221c81779aad552d50a195a6753a619a))
- **cron:** use consistent "success" status for completed jobs ([4f4b1e4](https://github.com/oxicrab/oxicrab/commit/4f4b1e41edaffb1bf7ca0e59b721c2f561e9dd01))
- harden transcription, reddit, and tmux tools ([58b28d0](https://github.com/oxicrab/oxicrab/commit/58b28d01c4109b8fdb66ee566fa5ddee86ffe560))
- harden media and todoist tools ([d8e9f3c](https://github.com/oxicrab/oxicrab/commit/d8e9f3ca9c4c0f0227aee5c124fe666fe3a4c215))
- harden Gmail, Calendar, and Obsidian tools ([60543bb](https://github.com/oxicrab/oxicrab/commit/60543bb35d5b2eed54535daebbf452111c30518e))
- harden truncation, image generation, and error types ([d8a0c3d](https://github.com/oxicrab/oxicrab/commit/d8a0c3d17f544574ecd3a4e2558e977a59597522))
- harden memory indexer and channel infrastructure ([3ade073](https://github.com/oxicrab/oxicrab/commit/3ade0735217b760347cc56dccada78334c5d1c49))
- **docs:** escape brackets and angle brackets in doc comments ([5495ae6](https://github.com/oxicrab/oxicrab/commit/5495ae63d534a6f1292c558f415068c38ae123ef))
- harden GitHub and browser tools ([d75c023](https://github.com/oxicrab/oxicrab/commit/d75c0236f949a98c8fb531d4d0f6315357a8f505))
- harden skills loader and web tools ([ffce550](https://github.com/oxicrab/oxicrab/commit/ffce55061e426f27ebe2de84ad23c318aea0ec80))
- harden MCP, heartbeat, and cognitive subsystems ([f68cf67](https://github.com/oxicrab/oxicrab/commit/f68cf67f8e5c53782d09a7ac4418fcef47d45af0))
- harden context builder and tool infrastructure ([1e6a8c9](https://github.com/oxicrab/oxicrab/commit/1e6a8c9483bf6e599e23d2295db4abbfb86c2a2e))
- harden auth and pairing subsystems ([d1dd61d](https://github.com/oxicrab/oxicrab/commit/d1dd61d5685fc762dcbebbaf885077b15506f7aa))
- harden shell tool and subagent subsystems ([b4df0d4](https://github.com/oxicrab/oxicrab/commit/b4df0d473bb276f06e58cf93e5ccefebb5744061))
- harden bus and channels subsystems ([a911449](https://github.com/oxicrab/oxicrab/commit/a911449adcc6a9414a970bd622c49292c85cf596))
- harden memory and utils subsystems ([432be16](https://github.com/oxicrab/oxicrab/commit/432be1653404543d97bd5b529ceb1ed3e15413c4))
- harden safety and compaction subsystems ([c4aa91b](https://github.com/oxicrab/oxicrab/commit/c4aa91b1bf75b85606009b7578e3196664195719))
- **oauth:** retry with refreshed token on 401 ([58944b5](https://github.com/oxicrab/oxicrab/commit/58944b592661d6ba13635a9f4cdd3ae89ce1cd2f))
- harden session and config, restructure large files ([1a898dd](https://github.com/oxicrab/oxicrab/commit/1a898ddf913985cf90fb10546e94bc5f51fed96b))
- harden providers and cron subsystems ([bd7e4ac](https://github.com/oxicrab/oxicrab/commit/bd7e4ac44c48d92ee94700318821f67b1c0d7804))
- harden agent loop, cost guard, and memory lock safety ([3a98a89](https://github.com/oxicrab/oxicrab/commit/3a98a8927b116af9837c6beeb571cedb9d8eeeac))
- **cost-guard:** always instantiate CostGuard for cost tracking ([3a16155](https://github.com/oxicrab/oxicrab/commit/3a16155f258f066b9373f600af9bc179aab63466))


### Maintenance
- **fmt:** ran cargo fmt ([6165303](https://github.com/oxicrab/oxicrab/commit/61653032165748c5b3a52b6834930749698091ff))

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
- add package build and validation jobs ([7fa8b2f](https://github.com/oxicrab/oxicrab/commit/7fa8b2ff95231985c18c3b77f55295c028009986))
- Package validation ([aca7a4e](https://github.com/oxicrab/oxicrab/commit/aca7a4e6fcb4e326d4720a9ddad49665a01ca9e8))


### Changed
- remove dead code and deduplicate provider HTTP clients ([77aa8f5](https://github.com/oxicrab/oxicrab/commit/77aa8f54e48ad908533af3982836aa307f386cbe))
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
- **ci:** restructured CI jobs again and fixed rpm build ([2e7fe6d](https://github.com/oxicrab/oxicrab/commit/2e7fe6d11b7431e7170a0fa7c21a5ff8fb9a6a3b))
- **ci:** restructured CI jobs to reduce compilation cycles ([61bda3a](https://github.com/oxicrab/oxicrab/commit/61bda3a52bcf85c0255cfbbcab2b202a168c533b))
- **deb:** resolve lintian errors failing CI package validation ([52e1c0c](https://github.com/oxicrab/oxicrab/commit/52e1c0c52d234c8543fe8dc053b1091a338d7520))
- address 15 quality issues from codebase review ([ff2367b](https://github.com/oxicrab/oxicrab/commit/ff2367b3ac12a10c84ce931dfa801dc6d80a86e0))
- **release:** Fixed up changelog to use a better format ([b3e3a54](https://github.com/oxicrab/oxicrab/commit/b3e3a5472e226f0063c3f367bf3078391ad96e13))
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


