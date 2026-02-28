# Changelog

All notable changes to this project will be documented in this file.

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


