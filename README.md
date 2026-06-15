# SeekerMail Desktop

> **本地优先的桌面邮件客户端** —— 每个邮箱账户都是一名 **AI 数字员工**(有角色、授权级别与决策范围),由本地语义向量索引 **GTE(Global Tactical Engine)** 驱动。
>
> **A local-first desktop email client** where every account is an **AI "digital employee"** — with a role, an authorization level, and a decision scope — powered by a local semantic vector index, the **GTE (Global Tactical Engine)**.

> **状态 / Status:** `pre-v0.1` — 设计与规格已完成,**产品代码尚未开始**;实现自 v0.1 起。
> Design & specs are complete; **product code has not started yet**. Implementation begins at v0.1.

---

## 技术栈 / Tech Stack

| 层 / Layer | 选型 / Choice |
| --- | --- |
| Shell | Tauri 2.x |
| 后端 / Backend | Rust |
| 前端 / Frontend | React 18 + TypeScript |
| 本地索引 / Local index | LanceDB(向量 / vector) |
| 形态 / Form | 桌面应用,本地优先,数据留在设备 / Desktop app, local-first, data stays on device |

---

## 仓库结构 / Repository Layout

```
seekermail-desktop/        ← 本仓库:仅代码,公开上 GitHub / this repo: code only, public on GitHub
├── CLAUDE.md              ← 写代码时的 AI 上下文 / AI context for coding
├── README.md              ← 本文件 / this file
├── .gitignore
├── src-tauri/             ← Rust 后端 + Tauri 配置 / Rust backend + Tauri config (from v0.1)
└── src/                   ← React 前端 / React frontend (from v0.1)
```

> 📁 **产品文档、规格与设计稿不在本仓库**,放在独立的私有知识库 `seekermail-desktop-2026/`(不上传 GitHub)。
> Product docs, specs, and design drafts are **not** in this repo — they live in the separate private knowledge base `seekermail-desktop-2026/` (never pushed to GitHub).

---

## 构建与运行 / Build & Run

> 构建脚本与依赖将在 v0.1 工程骨架落地时补全。届时这里会列出 `prerequisites`、`npm install`、`cargo tauri dev` 等命令。
> Build scripts and dependencies will be filled in when the v0.1 engineering skeleton lands. Prerequisites and the `npm install` / `cargo tauri dev` workflow will be documented here at that point.

参考工程规格 / Engineering specs to follow (in the knowledge base):

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

v1.0 是**首次公开 + 商用首发**;v0.1–v0.7 为内部/封闭验证(不公开、不收费、不签 SLA)。
v1.0 is the **first public + commercial release**; v0.1–v0.7 are internal/closed validation (not public, no charging, no SLA).

完整规划见知识库 / Full plan in the knowledge base: `../seekermail-desktop-2026/docs/planning/01_VERSION_ROADMAP.md`。

---

## 许可 / License

待定 —— 商用产品,正式发布前确定并补 `LICENSE`。
TBD — commercial product; the `LICENSE` will be finalized and added before public release.
