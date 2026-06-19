# SeekerMail Desktop

> **本地优先的桌面邮件客户端** —— 每个邮箱账户都是一名 **AI 数字员工**(有角色、授权级别与决策范围),由本地语义向量索引 **GTE(Global Tactical Engine)** 驱动。
>
> **A local-first desktop email client** where every account is an **AI "digital employee"** — with a role, an authorization level, and a decision scope — powered by a local semantic vector index, the **GTE (Global Tactical Engine)**.

> **状态 / Status:** `开发中 / In development` — `package.json` 版本 `0.1.0`,**尚未公开发布**。后端与前端已大量落地(IMAP 同步、本地索引、AI 流水线、自带 AI 供应商、搜索、导出);邮件收发管线刚刚端到端打通。
> Version `0.1.0`; **not yet publicly released.** Substantial backend + frontend already exist (IMAP sync, the local index, the AI pipeline, BYO AI providers, search, export); the mail send/receive pipeline was just wired end-to-end.

> ⚠️ **关于构建 / Build note:** 默认编译跑的是**离线替身**(不收发真实邮件)。真实 IMAP/SMTP/OAuth 需带特性开关 `--features live-net` 构建——项目脚本 `pnpm tauri:dev` / `pnpm tauri:build` **已默认带上**。一个不带该开关的纯 `cargo build` 会**拒绝发信**(而不是假装成功)。
> The default compile runs **offline stand-ins** (no real mail). Real IMAP/SMTP/OAuth need `--features live-net` — the `pnpm tauri:dev` / `pnpm tauri:build` scripts pass it for you. A bare `cargo build` without the flag will **refuse to send** (rather than silently pretend).

---

## 技术栈 / Tech Stack

| 层 / Layer | 选型 / Choice |
| --- | --- |
| Shell | Tauri 2.x |
| 后端 / Backend | Rust(`sqlx` + SQLite,`async-imap`,`lettre`,`reqwest`)|
| 前端 / Frontend | React 18 + TypeScript（Vite、TanStack Query、Zustand、Tailwind）|
| 本地索引 / Local index | **GTE** —— 本地语义向量索引（当前为快照上的余弦暴力检索;LanceDB 级 ANN 为扩展目标 / cosine over a local snapshot today; a LanceDB-class ANN store is the scaling target）|
| 本地模型 / Local models | 可选 / optional：ONNX 嵌入（`--features local-embed`）、GGUF 本地大模型（`--features local-llm`）|
| 形态 / Form | 桌面应用,本地优先,数据留在设备 / Desktop app, local-first, data stays on device |

---

## 仓库结构 / Repository Layout

```
seekermail-desktop/        ← 本仓库:仅代码,公开上 GitHub / this repo: code only, public on GitHub
├── CLAUDE.md              ← 写代码时的 AI 上下文 / AI context for coding
├── README.md              ← 本文件 / this file
├── package.json           ← 前端脚本与依赖 / frontend scripts & deps
├── src/                   ← React 前端 / React frontend
├── src-tauri/             ← Rust 后端 + Tauri 配置 + 迁移 / Rust backend + Tauri config + migrations
│   ├── src/               ← Rust 模块(imap / storage / ai / search / …) / Rust modules
│   └── migrations/        ← SQLite 迁移(前向只进) / forward-only SQLite migrations
├── packages/shared/       ← Rust→TS 生成的类型 / generated Rust→TS bindings
└── xtask/                 ← 基准与安全门工具 / bench + safety-gate tooling
```

> 📁 **产品文档、规格与设计稿不在本仓库**,放在独立的私有知识库 `seekermail-desktop-2026/`(不上传 GitHub)。
> Product docs, specs, and design drafts are **not** in this repo — they live in the separate private knowledge base `seekermail-desktop-2026/` (never pushed to GitHub).

---

## 构建与运行 / Build & Run

**前置 / Prerequisites:** Node `>=20 <23` · pnpm `9` · Rust(stable 工具链 / stable toolchain)· 各平台的 Tauri 依赖 / Tauri OS prerequisites。

```bash
pnpm install              # 安装前端依赖 / install frontend deps
pnpm setup:model          # 拉取本地嵌入模型(可选)/ fetch the local embedding model (optional)

pnpm tauri:dev            # 跑桌面 App(带 --features live-net,可收发真实邮件)/ desktop app, real mail
pnpm dev                  # 只跑前端(Vite,IPC 走 mock)/ frontend only (Vite, mocked IPC)

pnpm check                # tsc + eslint + prettier
pnpm test                 # 前端单测(Vitest)/ frontend unit tests
pnpm gen:types            # 重新生成 Rust→TS 类型绑定 / regenerate the TS bindings

pnpm tauri:build          # 生产构建(带 --features live-net)/ production build
```

> 后端测试在 Mac 上跑 / Backend tests on a Mac: `cargo test --manifest-path src-tauri/Cargo.toml`。
> 验证真实网络路径需带特性 / To type-check the real network path: `cargo check --features live-net`。

参考工程规格 / Engineering specs (in the knowledge base):

- `../seekermail-desktop-2026/docs/dev/01_DATABASE_SCHEMA.md`
- `../seekermail-desktop-2026/docs/dev/02_IPC_COMMAND_CONTRACTS.md`
- `../seekermail-desktop-2026/docs/dev/03_RUST_MODULE_INTERFACES.md`
- `../seekermail-desktop-2026/docs/dev/05_PACKAGING_AND_NOTARIZATION.md`

---

## 语言规则 / Language Rules

- 代码、注释、标识符、UI 文案一律 **英文**(i18n 功能除外)。
  Code, comments, identifiers, and UI copy are **English only** (except the i18n feature).
- 与团队/用户沟通用**中文**。Communication with the team/user is in **Chinese**.

---

## 版本路线 / Version Roadmap

`v0.1 Internal → v0.4 Beta → v0.5 → v0.6 → v0.7 RC → v1.0 GA`

v1.0 是**首次公开 + 商用首发**;v0.1–v0.7 为内部/封闭验证(不公开、不收费、不签 SLA)。当前处于该路线的早期实现阶段(版本号 `0.1.0`),`docs/releases/` 下的 `v1.0.0_*` 文件是**前瞻草稿/门禁证据模板**,并非已发布版本。
v1.0 is the **first public + commercial release**; v0.1–v0.7 are internal/closed validation. The project is at an early implementation stage of that roadmap (version `0.1.0`); the `v1.0.0_*` files under `docs/releases/` are **forward-looking drafts / gate-evidence templates**, not a shipped release.

完整规划见知识库 / Full plan in the knowledge base: `../seekermail-desktop-2026/docs/planning/01_VERSION_ROADMAP.md`。

---

## 许可 / License

待定 —— 商用产品,正式发布前确定并补 `LICENSE`。
TBD — commercial product; the `LICENSE` will be finalized and added before public release.
