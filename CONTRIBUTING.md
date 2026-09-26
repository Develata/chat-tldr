# 协作指南（CONTRIBUTING）

> 写给第一次用 Git 和 GitHub 协作的同学。照着步骤做就行。遇到任何看不懂的情况，先停下来，在群里问 @Develata，不要自己尝试“修复” Git。

## 三条铁律

1. **不要直接改 `main` 分支**。每项工作都开一个新分支，通过 PR 合并。
2. **不要 rebase，不要 force push**（`git rebase`、`git push -f` 一律不用）。分支落后时点网页上的 “Update branch” 按钮。
3. **只改当前任务分配的目录**（最新分工见 [TEAM_ASSIGNMENTS](docs/TEAM_ASSIGNMENTS.md)，文件归属见 [FILE_LAYOUT](docs/FILE_LAYOUT.md)）：
   - QCE 管理任务：`apps/qce-manager/`；QCE JSON 导入适配器 `crates/qce/` 已由 Codex 主线承担。
   - GUI 设计任务：`docs/ui/`；需要参与界面实现时，由任务明确分配 `apps/gui/` 中的范围。
   - 合成数据、标注与报告协作：按任务分别放 `fixtures/`、`eval/synthetic/`、`reports/`；真实数据只放 `eval/private/`。
   - core、engine、CLI、导入适配器及主线工具由 Codex 实现、@Develata 审核。`docs/tasks/` 中的早期 A/B/C 名称不代表当前已经分派。

   `crates/core`（共享类型）由 @Develata 决策、授权 Codex 实现。其他同学需要改动时先说明接口需求；CODEOWNERS 用于请求审核，不是写权限控制。

另外：**绝不提交真实聊天记录和 API key**。真实数据放 `eval/private/` 或 `private/`（已被 Git 忽略），key 只放在环境变量里。

---

## 第 0 步：一次性准备（第 1 天上午完成）

1. 安装 [Git](https://git-scm.com/)（Windows 安装时一路默认即可，会附带 Git Bash）。
2. 安装 Rust：按 [rustup.rs](https://rustup.rs/) 的说明安装，完成后在终端运行 `cargo --version`，能看到版本号就说明安装成功。
3. 注册 GitHub 账号，把用户名告诉 @Develata。收到仓库邀请邮件后点 **Accept invitation**。
4. 告诉 Git 你是谁（只需要做一次）：

   ```bash
   git config --global user.name "你的 GitHub 用户名"
   git config --global user.email "你的 GitHub 邮箱"
   ```

5. 登录 GitHub：第一次 `git push` 时会弹出浏览器窗口让你登录，按提示授权即可。

> 以下命令都在 **Git Bash** 或 **VS Code 的终端**里运行。Windows 自带的旧版 PowerShell 5 不支持 `&&`，请改用 Git Bash 或 PowerShell 7。

## 第 1 步：克隆仓库（只做一次）

```bash
git clone https://github.com/Develata/chat-tldr.git
cd chat-tldr
```

**直接克隆主仓库，不要 fork。**

## 第 2 步：每次开始一项新工作

```bash
git switch main
git pull
git switch -c a/qce-manager
```

- 前两行：把本地的 main 更新到最新。
- 第三行：从最新的 main 建一个新分支。分支名格式为 `<你的字母>/<简短英文描述>`，例如 `a/qce-manager`、`b/gui-design`、`c/synthetic-cases`。
- **一个分支只做一件事**，做完合并后就不再使用。

## 第 3 步：写代码，并在提交前自检

在仓库根目录运行这**一条命令**（格式化 + 静态检查 + 测试）：

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

- 全部通过，最后没有红色的 `error` → 可以提交。
- 有报错 → 先修好再提交。看不懂报错的，见第 7 步。

## 第 4 步：提交

```bash
git status
git add apps/qce-manager
git commit -m "qce-manager: 补充导出流程与失败处理"
```

- `git status` 列出改动过的文件。**检查一遍**：里面不应该有真实聊天记录、`.env`、数据库文件。
- `git add` 后面写**你自己负责的目录**，不要用 `git add .` 或 `git add -A`，以免把无关文件带进去。
- 提交说明格式为 `模块: 做了什么`，中文即可。
- 可以多次提交，最后合并时会自动压成一个提交。

## 第 5 步：推送

```bash
git push -u origin a/qce-manager
```

第一次推送这个分支时带上 `-u origin <分支名>`，之后同一个分支再推送只需要 `git push`。

## 第 6 步：开 PR（Pull Request）

1. 推送成功后，打开仓库网页，会看到黄色提示条 **“Compare & pull request”**，点它。
2. 标题写清楚做了什么，正文按模板填写（模板会自动出现）。在正文中写 `Closes #<任务 issue 编号>`，合并后 issue 会自动关闭。
3. 点 **Create pull request**。
4. 等待两件事：
   - CI 检查（`fmt`、`clippy`、`test`）全部变绿（✅）；
   - @Develata 看过代码。
5. 两者都满足后，由 @Develata 点 **Squash and merge**。**你不需要、也不要自己点合并**，即使按钮可以点。

审核意见要求修改时：在同一个分支上继续改，然后重复第 3–5 步（`git push` 即可），PR 会自动更新。

## 第 7 步：看懂 CI 报错

1. 在 PR 页面底部找到红色 ❌ 的检查项，点右侧的 **Details**。
2. 左侧会显示是哪一步失败了：
   - `fmt` 失败：运行 `cargo fmt --all`，然后提交、推送。
   - `clippy` 失败：代码风格或潜在错误的警告，需要按提示修改。
   - `test` 失败：有测试没通过。
3. 展开失败的步骤，找到带 `error` 的几行，**连同上下 20 行一起复制**。
4. 把报错交给 AI 编程工具（例如 Claude Code、Cursor、Copilot）修复，提示词可以这样写：

   > 这是 GitHub Actions 中 `cargo clippy --workspace --all-targets -- -D warnings` 的报错。请只修改 `crates/qce/` 目录下的文件来修复它，不要改其他目录，也不要修改测试的预期结果：
   > （粘贴报错）

5. 修好后在本地运行第 3 步的自检命令，通过后再提交、推送。

## 第 8 步：分支落后于 main 时

PR 页面出现 **“This branch is out-of-date with the base branch”** 时：

- 点 **Update branch** 按钮（如果有下拉菜单，选 **Update with merge commit**，**不要**选 rebase）。
- 然后在本地运行 `git pull`，把网页上的更新同步下来，再继续工作。

如果提示有**冲突（conflict）**，不要自己处理，在群里 @Develata。

## 第 9 步：合并之后

```bash
git switch main
git pull
```

旧分支不再使用。下一项工作从第 2 步重新开始。

---

## 常见问题

**Q：不小心在 main 上改了代码，还没有提交？**
运行 `git switch -c <新分支名>`，改动会跟着你到新分支上，然后照常提交。

**Q：`git push` 被拒绝，提示 `rejected`？**
通常是网页上点过 Update branch，而本地还没有同步。先运行 `git pull`，再 `git push`。如果 `git pull` 提示冲突，在群里问。

**Q：我需要的类型在 `crates/core` 里没有？**
不要自己加。在群里或 issue 中说明需要什么，@Develata 会修改类型，并按规则升级 `schema_version`。

**Q：能不能改别人目录里的一个小 bug？**
不能直接改。开一个 issue，或在群里告诉负责人。

**Q：Linux 上编译 GUI 报缺少库？**
安装：`sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev`。
