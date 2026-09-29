# 图片预览换用 Annotorious + OpenSeadragon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `png/jpg/jpeg/webp/bmp/ico` 图片预览换用一个新的 `dozer://image-annotate/` webview host(OpenSeadragon 缩放/平移 + Annotorious 标注,矩形/点/多边形 + 文字备注),彻底替代这几个扩展名当前依赖的 Flyfish 渲染管线;`gif/tif/tiff` 及所有非图片媒体保持现状不变。标注结果本轮只放内存,不落盘、不发给 agent。

**Architecture:** 新增一个独立 `dozer://image-annotate/` 协议命名空间(照抄 `editor`/`json-editor` 的"自己的静态资源根 + `__file__` 白名单端点"模式,而不是 `html/` 的双层 sandbox iframe 模式——那个模式是为不受信的用户 HTML 准备的,我们这里渲染的是自己写的可信 host 页)。路由分流发生在现有的唯一渲染 URL 决策点 `preview::preview_url`,不新增 `PreviewKind`。Host ↔ Rust 走现有的 `WebviewEnvelope`/`HostBinding` T9 机制,新起一套 `ImageAnnotateEvent`,不复用 `FlyfishEvent`。

**Tech Stack:** OpenSeadragon(BSD-3-Clause)+ `@annotorious/openseadragon`(BSD-3-Clause,内部依赖 `@annotorious/annotorious`),两者都以 jsdelivr `+esm` 打包成单文件 ESM 后 vendor 进仓库,不引入 Node 构建流程。Rust 侧沿用现有 `wry` 自定义协议 + `serde`/`serde_json`。

**Spec:** `docs/superpowers/specs/2026-09-29-image-annotate-viewer-design.md`

## Global Constraints

- 新查看器仅覆盖 `png/jpg/jpeg/webp/bmp/ico`;`gif/tif/tiff` 与 `router.rs::is_known_media_extension` 里所有非图片扩展名必须继续走 `dozer://flyfish/`,不得改动该函数的既有匹配列表。
- 标注结果(`ImageAnnotateEvent::AnnotationsChanged`)本轮只放 `PreviewTab.image_annotations` 内存字段,不落盘、不发给 agent、不定义任何持久化/W3C 序列化格式——Rust 侧把每条标注当不透明 `serde_json::Value` 存取。
- 新 host 的 CSP 必须"无网络":`default-src 'none'`、`script-src 'self'`,不允许任何 `http://`/`https://` 引用;所有 JS/CSS 必须是本地 vendor 文件。
- 不得在 `cargo build` 流程里引入 Node/Python 工具链;vendor 文件是一次性用 `curl` 拉取的预构建产物,提交进仓库。
- Host → Rust 消息必须走 `WebviewEnvelope`/`HostBinding` 校验(project/panel/tab/doc 全部一致才接受),JS 不能自报归属——这是仓库里所有 webview host 的硬性规则,不为这个新 host 开例外。

## Review Focus

- `.ico`/`.bmp` 在 WebKit 里解码失败时,必须落到 `Failed` → 统一 fallback 页,不能白屏或无限 loading——Task 7 的 `apply_image_annotate_event_document_loaded_with_error_transitions_failed`/`apply_image_annotate_event_failed_transitions_failed_state` 测试覆盖 Rust 侧状态机部分;JS 侧解码失败是否真的触发 `error` 事件是人工验收清单第 7 项(见 spec §7),Task 9 执行。
- 恶意或损坏的 IPC payload(非 JSON、超大消息、`project_id`/`tab_id`/`document_id` 不匹配)必须被拒绝且不 panic——Task 6 的 `parse_image_annotate_event` 测试覆盖(镜像 `parse_flyfish_event` 现有测试)。
- 事件到达时对应 `tab_id` 的 tab 已经被关闭/替换——`apply_image_annotate_event` 找不到 tab 必须静默返回,不能 panic——Task 7 的 `apply_image_annotate_event_ignores_unknown_tab_id` 测试覆盖。
- 迟到的 `DocumentLoaded`/`Failed` ACK(用户已经重新加载过、`load_state` 不再 active)不能把新一轮加载错误地标记完成——Task 7 的 `apply_image_annotate_event_stale_document_loaded_does_not_finish_new_load` 测试覆盖(复用现有 `load_state.is_active()` 闸门)。
- 0 字节或损坏的图片文件被打开——`serve_allowlisted_file` 读到空/损坏字节后原样交给 host,host 侧 OpenSeadragon 解码失败必须报 `Failed` 而不是挂起;Rust 侧状态机部分由 Task 7 的 `Failed`/`DocumentLoaded{error}` 测试覆盖,JS 侧解码失败是否真的触发 `error` 事件是人工验收清单第 7 项,Task 9 执行(见 spec §7)。

---

### Task 1: Vendor OpenSeadragon + Annotorious-OpenSeadragon 静态资源

**Files:**
- Create: `crates/dozer-app/assets/image-annotate/vendor/openseadragon.esm.js`
- Create: `crates/dozer-app/assets/image-annotate/vendor/annotorious-openseadragon.esm.js`
- Create: `crates/dozer-app/assets/image-annotate/VENDORED_VERSION`
- Test: `crates/dozer-app/src/assets.rs`(新增测试,追加到现有 `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:`crates/dozer-app/assets/image-annotate/vendor/*.esm.js` 两个自包含 ESM 文件(供 Task 3 的 `host.html` 用 `<script type="module">` 动态 `import`)。

- [ ] **Step 1: 查询两个包的可用版本(jsdelivr data API,纯 curl,不装 Node)**

```bash
curl -s https://data.jsdelivr.com/v1/packages/npm/openseadragon | python3 -c "import json,sys; d=json.load(sys.stdin); print([v['version'] for v in d['versions'] if '-' not in v['version']][:5])"
curl -s https://data.jsdelivr.com/v1/packages/npm/@annotorious/openseadragon | python3 -c "import json,sys; d=json.load(sys.stdin); print([v['version'] for v in d['versions'] if '-' not in v['version']][:5])"
```

记下两边输出的第一个(最新非预发布)版本号,后续步骤里的 `<OSD_VERSION>`/`<ANNO_VERSION>` 替换成实际值。

- [ ] **Step 2: 下载两个自包含 ESM bundle**

```bash
mkdir -p crates/dozer-app/assets/image-annotate/vendor
curl -sL "https://cdn.jsdelivr.net/npm/openseadragon@<OSD_VERSION>/+esm" \
  -o crates/dozer-app/assets/image-annotate/vendor/openseadragon.esm.js
curl -sL "https://cdn.jsdelivr.net/npm/@annotorious/openseadragon@<ANNO_VERSION>/+esm" \
  -o crates/dozer-app/assets/image-annotate/vendor/annotorious-openseadragon.esm.js
```

- [ ] **Step 3: 校验下载产物离线安全(无残留外部 import/引用)**

```bash
grep -E "from *[\"']https?://|import\\(.*https?://" crates/dozer-app/assets/image-annotate/vendor/*.esm.js && echo "FAIL: 含外部引用" || echo "OK: 无外部引用"
```

若报 FAIL,说明 jsdelivr `+esm` 没能把某个依赖内联(常见于依赖了 Node 内置模块或极大的可选依赖),需要改用该依赖单独的 `+esm` 地址递归拉取、或退回上游仓库自带的 `dist/` UMD 产物重新评估——这一步必须在提交前解决,不能把带外部 fetch 的文件提交进仓库(直接违反"无网络"CSP 约束)。

- [ ] **Step 4: 记录导出符号(供 Task 3 写 import 语句用)**

```bash
grep -o 'export[^;{]*' crates/dozer-app/assets/image-annotate/vendor/openseadragon.esm.js | head -5
grep -o 'export[^;{]*' crates/dozer-app/assets/image-annotate/vendor/annotorious-openseadragon.esm.js | head -5
```

把两边真实的导出形式(`export default ...` 还是 `export {...}`,以及 `createOSDAnnotator` 之类具名导出是否存在)记下来——Task 3 的 `import` 语句要跟这里看到的真实符号一致,不能假设。

- [ ] **Step 5: 写版本记录文件**

```
crates/dozer-app/assets/image-annotate/VENDORED_VERSION
```

```
openseadragon @<OSD_VERSION>(BSD-3-Clause),经 https://cdn.jsdelivr.net/npm/openseadragon@<OSD_VERSION>/+esm 打包成自包含 ESM
@annotorious/openseadragon @<ANNO_VERSION>(BSD-3-Clause,内联 @annotorious/annotorious 核心),经 https://cdn.jsdelivr.net/npm/@annotorious/openseadragon@<ANNO_VERSION>/+esm 打包成自包含 ESM
均为一次性 curl 拉取的预构建产物,不接入 Node 构建流程;升级时重跑 Task 1 Step 1-4。
```

- [ ] **Step 6: 加资产存在性 + 离线安全测试(镜像 `editor_bundle_assets_are_present`/`editor_index_has_strict_csp_and_no_external_refs`)**

追加到 `crates/dozer-app/src/assets.rs` 的 `#[cfg(test)] mod tests`:

```rust
    /// Task 1:vendor 的 image-annotate 依赖必须存在且非空(防止忘记提交)。
    #[test]
    fn image_annotate_vendor_assets_are_present() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/image-annotate/vendor"
        ));
        for f in ["openseadragon.esm.js", "annotorious-openseadragon.esm.js"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 image-annotate vendor 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "image-annotate vendor 产物为空: {f}"
            );
        }
    }

    /// Task 1:vendor 产物不得残留外部网络引用(CSP "无网络" 约束的前提)。
    #[test]
    fn image_annotate_vendor_assets_have_no_external_refs() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/image-annotate/vendor"
        ));
        for f in ["openseadragon.esm.js", "annotorious-openseadragon.esm.js"] {
            let js = std::fs::read_to_string(root.join(f)).expect("读 vendor 产物");
            assert!(
                !js.contains("http://") && !js.contains("https://"),
                "{f} 不得引用外部 URL(离线约束)"
            );
        }
    }
```

- [ ] **Step 7: 跑测试确认通过**

Run: `cargo test -p dozer-app assets::tests::image_annotate_vendor -- --nocapture`
Expected: 两个新测试 PASS。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/assets/image-annotate/vendor crates/dozer-app/assets/image-annotate/VENDORED_VERSION crates/dozer-app/src/assets.rs
git commit -m "chore(image-annotate): vendor OpenSeadragon + Annotorious-OpenSeadragon bundles"
```

---

### Task 2: `dozer://image-annotate/` 协议命名空间(host.html 服务 + `__file__` 端点)

**Files:**
- Modify: `crates/dozer-app/src/assets.rs`
- Create: `crates/dozer-app/src/image_annotate_host.html`(编译期 `include_str!` 嵌入,内容在 Task 3 填充;本任务先放一个最小占位 `<!doctype html>` 骨架,Task 3 覆盖为完整内容——这不是"留 TODO 不实现",而是这两个任务本就分属"协议路由"与"前端交互"两个可独立评审的关注点,骨架本身是合法的、可编译可测的中间态)
- Test: `crates/dozer-app/src/assets.rs`

**Interfaces:**
- Consumes: `serve_vendored`(已存在)、`serve_allowlisted_file`(已存在,`fn serve_allowlisted_file(encoded: &str, allowed: &HashSet<PathBuf>) -> ProtocolReply`)。
- Produces: `handle_protocol` 新增对 `dozer://image-annotate/` 前缀的分发,供 Task 5 的 `image_annotate_url` 构造的 URL 实际可被 wry 加载。

- [ ] **Step 1: 写最小占位 host 页**

```html
<!-- crates/dozer-app/src/image_annotate_host.html -->
<!doctype html>
<html><head><meta charset="utf-8"></head><body></body></html>
```

- [ ] **Step 2: 写失败测试(命名空间尚未接线,应 404)**

追加到 `crates/dozer-app/src/assets.rs` 的 `#[cfg(test)] mod tests`:

```rust
    /// Task 2:`dozer://image-annotate/host.html` 服务编译期内嵌页面。
    #[test]
    fn serves_image_annotate_host() {
        let root = scratch();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            None,
            "dozer://image-annotate/host.html",
        );
        assert_eq!((r.status, r.mime), (200, "text/html"));
    }

    /// Task 2:`__file__` 端点复用现有白名单读取,未在白名单内 404、在白名单内 200。
    #[test]
    fn image_annotate_file_endpoint_requires_allowlist() {
        let root = scratch();
        let f = root.join("photo.png");
        fs::write(&f, b"\x89PNG\r\n").unwrap();
        let uri = format!("dozer://image-annotate/__file__{}", f.to_string_lossy());
        assert_eq!(
            handle_protocol(&root, &HashSet::new(), None, &uri).status,
            404
        );
        let mut allowed = HashSet::new();
        allowed.insert(f.clone());
        let r = handle_protocol(&root, &allowed, None, &uri);
        assert_eq!((r.status, r.mime), (200, "image/png"));
    }

    /// Task 2:命名空间拒绝路径穿越(同 editor/json-editor/tabular 既有覆盖)。
    #[test]
    fn image_annotate_rejects_traversal_and_unknown() {
        let root = scratch();
        for uri in [
            "dozer://image-annotate/../flyfish/host.html",
            "dozer://image-annotate/%2e%2e/etc/passwd",
            "dozer://image-annotate/nope.js",
        ] {
            assert_eq!(
                handle_protocol(&root, &HashSet::new(), None, uri).status,
                404,
                "{uri}"
            );
        }
    }
```

- [ ] **Step 3: 跑测试确认失败(命名空间还没接线,落到 `handle_protocol` 末尾的 flyfish fallback 或 404)**

Run: `cargo test -p dozer-app assets::tests::serves_image_annotate_host assets::tests::image_annotate_file_endpoint_requires_allowlist -- --nocapture`
Expected: FAIL(`serves_image_annotate_host` 拿到的不是 200,或 `image_annotate_file_endpoint_requires_allowlist` 因命名空间被 `flyfish/` fallback 误吞而行为不对)。

- [ ] **Step 4: 实现协议分发**

在 `crates/dozer-app/src/assets.rs` 里,`tabular_host_root_for` 函数之后新增:

```rust
/// image-annotate host(OpenSeadragon + Annotorious 图片查看/标注)静态资源根
/// = flyfish 根的兄弟目录 `image-annotate`。同 `editor_root_for`。
fn image_annotate_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("image-annotate")
}
```

在 `handle_protocol` 里,`html/` 分支(`if let Some(path) = rest.strip_prefix("html/")` 那一段)之后、`flyfish/` fallback 之前插入:

```rust
    // image-annotate host(OpenSeadragon + Annotorious):页面编译期内嵌;
    // `__file__` 走 `serve_allowlisted_file`——这个 host 只需要精确读取
    // "当前打开的这一张图片"本身,不像 HTML 预览需要相对资源子树访问。
    if let Some(path) = rest.strip_prefix("image-annotate/") {
        if path == "host.html" {
            return ProtocolReply {
                status: 200,
                mime: "text/html",
                body: include_str!("image_annotate_host.html").as_bytes().to_vec(),
            };
        }
        if let Some(encoded) = path.strip_prefix("__file__") {
            return serve_allowlisted_file(encoded, allowed);
        }
        return serve_vendored(&image_annotate_root_for(assets_root), path);
    }
```

（放在 `html/` 分支之后、`let Some(path) = rest.strip_prefix("flyfish/") ...` 之前。）

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p dozer-app assets::tests::serves_image_annotate_host assets::tests::image_annotate_file_endpoint_requires_allowlist assets::tests::image_annotate_rejects_traversal_and_unknown -- --nocapture`
Expected: 三个测试 PASS。

- [ ] **Step 6: 跑现有 assets.rs 全部测试确认无回归**

Run: `cargo test -p dozer-app assets:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/assets.rs crates/dozer-app/src/image_annotate_host.html
git commit -m "feat(image-annotate): add dozer://image-annotate/ protocol namespace"
```

---

### Task 3: 图片标注 host 页(OpenSeadragon + Annotorious 集成)

**Files:**
- Modify: `crates/dozer-app/src/image_annotate_host.html`(覆盖 Task 2 的占位内容为完整实现)
- Test: `crates/dozer-app/src/assets.rs`

**Interfaces:**
- Consumes: Task 1 vendor 的 `vendor/openseadragon.esm.js`(`import OpenSeadragon from './vendor/openseadragon.esm.js'`,导出形式以 Task 1 Step 4 记录的真实符号为准,若非 `export default` 按实际调整)、`vendor/annotorious-openseadragon.esm.js`(`import { createOSDAnnotator } from './vendor/annotorious-openseadragon.esm.js'`,同样以 Task 1 Step 4 记录为准)。
- Consumes:`dozer://image-annotate/__file__<编码后的绝对路径>` 加载图片字节(Task 2 已接线)。
- Produces:`window.__dozerImageAnnotatePost(payload)` envelope 通道(Task 8 的 Rust IPC handler 消费)。

- [ ] **Step 1: 写内容测试(CSP/离线约束,镜像 `editor_index_has_strict_csp_and_no_external_refs`)**

追加到 `crates/dozer-app/src/assets.rs` 的 `#[cfg(test)] mod tests`:

```rust
    /// Task 3:host 页必须声明严格 CSP、无外部引用、引用了两个 vendor 文件。
    #[test]
    fn image_annotate_host_has_strict_csp_and_no_external_refs() {
        let html = include_str!("image_annotate_host.html");
        assert!(html.contains("Content-Security-Policy"), "必须声明 CSP");
        assert!(html.contains("default-src 'none'"), "CSP 必须以 default-src 'none' 起步");
        assert!(html.contains("script-src 'self'"), "脚本仅 self");
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "host.html 不得引用外部 URL(离线约束)"
        );
        assert!(html.contains("vendor/openseadragon.esm.js"));
        assert!(html.contains("vendor/annotorious-openseadragon.esm.js"));
        assert!(html.contains("__dozerImageAnnotatePost"));
    }
```

- [ ] **Step 2: 跑测试确认失败(占位页没有这些内容)**

Run: `cargo test -p dozer-app assets::tests::image_annotate_host_has_strict_csp_and_no_external_refs -- --nocapture`
Expected: FAIL。

- [ ] **Step 3: 实现完整 host 页**

```html
<!-- crates/dozer-app/src/image_annotate_host.html -->
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Dozer Image Annotate</title>
<meta http-equiv="Content-Security-Policy"
      content="default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'">
<style>
  html { --preview-bg: #0a0e16; --accent: #F2D94E; }
  html[data-theme="light"] { --preview-bg: #fef2e4; --accent: #F2D94E; }
  html, body { margin: 0; height: 100%; background: var(--preview-bg); overflow: hidden; }
  #viewer { position: absolute; inset: 0; }
  /* Annotorious 默认高亮色跟随 ByteBoy2077 金色(甲方动作专属)。 */
  .a9s-annotation .a9s-outer { stroke: var(--accent) !important; }
  .a9s-annotation .a9s-inner { stroke: var(--accent) !important; fill: transparent; }
  #toolbar {
    position: absolute; top: 8px; left: 8px; z-index: 10;
    display: flex; gap: 4px; background: rgba(0,0,0,0.35); border-radius: 6px; padding: 4px;
  }
  #toolbar button {
    background: transparent; border: 1px solid var(--accent); color: var(--accent);
    border-radius: 4px; padding: 4px 8px; font-size: 12px; cursor: pointer;
  }
  #toolbar button.active { background: var(--accent); color: #0a0e16; }
</style>
</head>
<body>
<div id="toolbar">
  <button id="tool-pan" class="active" data-tool="">平移</button>
  <button id="tool-rect" data-tool="rectangle">矩形</button>
  <button id="tool-point" data-tool="point">点</button>
  <button id="tool-polygon" data-tool="polygon">多边形</button>
</div>
<div id="viewer"></div>
<script type="module">
  import OpenSeadragon from './vendor/openseadragon.esm.js';
  import { createOSDAnnotator } from './vendor/annotorious-openseadragon.esm.js';

  const params = new URLSearchParams(location.search);
  const p = decodeURIComponent(params.get('p') || '');
  const theme = params.get('theme') === 'light' ? 'light' : 'dark';
  document.documentElement.setAttribute('data-theme', theme);

  // T9:host → Rust 的统一 envelope 通道,与 Flyfish 的 `__dozerFlyfishPost`
  // 平行但不共享——payload kind 集合不同(见 `ImageAnnotateEvent`)。
  function envelope(payload) {
    return JSON.stringify({
      protocol_version: 1,
      project_id: Number(params.get('proj') || 0) || 0,
      panel: params.get('panel') || 'files',
      tab_id: Number(params.get('tab') || 0) || 0,
      document_id: params.get('doc') || '',
      revision: 0,
      request_id: null,
      payload,
    });
  }
  function post(payload) {
    try { window.ipc.postMessage(envelope(payload)); } catch (_) {}
  }
  window.__dozerImageAnnotatePost = post;

  post({ kind: 'ready' });

  const viewer = OpenSeadragon({
    element: document.getElementById('viewer'),
    tileSources: { type: 'image', url: '__file__' + encodeURI(p) },
    showNavigationControl: true,
    gestureSettingsMouse: { clickToZoom: false, dblClickToZoom: true },
    prefixUrl: null,
  });

  viewer.addHandler('open', function () {
    post({ kind: 'document_loaded' });
  });
  viewer.addHandler('open-failed', function (e) {
    post({ kind: 'failed', message: (e && e.message) || '图片解码失败', recoverable: true });
  });

  const anno = createOSDAnnotator(viewer, { drawingEnabled: false });

  function snapshotAnnotations() {
    post({ kind: 'annotations_changed', annotations: anno.getAnnotations() });
  }
  anno.on('createAnnotation', snapshotAnnotations);
  anno.on('updateAnnotation', snapshotAnnotations);
  anno.on('deleteAnnotation', snapshotAnnotations);

  // 工具条:切换绘制工具 / 回到平移模式(默认态,拖拽 = OSD 平移,不与
  // 标注绘制手势冲突——见 spec §5 模式边界)。
  const buttons = Array.from(document.querySelectorAll('#toolbar button'));
  buttons.forEach(function (btn) {
    btn.addEventListener('click', function () {
      const tool = btn.getAttribute('data-tool');
      buttons.forEach(function (b) { b.classList.toggle('active', b === btn); });
      if (tool) {
        anno.setDrawingTool(tool);
        anno.setDrawingEnabled(true);
      } else {
        anno.setDrawingEnabled(false);
      }
    });
  });
</script>
</body>
</html>
```

（`import` 的两个符号名需要按 Task 1 Step 4 实际记录的导出形式核对调整;若 `annotorious-openseadragon.esm.js` 没有具名导出 `createOSDAnnotator` 而是挂在默认导出对象上,把这行改成 `import AnnoOSD from '...'; const { createOSDAnnotator } = AnnoOSD;`。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app assets::tests::image_annotate_host_has_strict_csp_and_no_external_refs -- --nocapture`
Expected: PASS。

- [ ] **Step 5: 跑 assets.rs 全部测试确认无回归**

Run: `cargo test -p dozer-app assets:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/image_annotate_host.html crates/dozer-app/src/assets.rs
git commit -m "feat(image-annotate): implement OpenSeadragon + Annotorious host page"
```

---

### Task 4: 路由扩展名判定 `is_image_annotate_extension`

**Files:**
- Modify: `crates/dozer-app/src/preview/router.rs`
- Test: `crates/dozer-app/src/preview/router.rs`(同文件 `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub(crate) fn is_image_annotate_extension(path: &Path) -> bool`,供 Task 5 的 `preview_url` 使用。

- [ ] **Step 1: 写失败测试**

追加到 `crates/dozer-app/src/preview/router.rs` 的 `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn image_annotate_extension_covers_browser_native_raster_formats() {
        for p in ["a.png", "b.jpg", "c.jpeg", "d.webp", "e.bmp", "f.ico", "g.PNG"] {
            assert!(is_image_annotate_extension(&PathBuf::from(p)), "{p}");
        }
    }

    #[test]
    fn image_annotate_extension_excludes_gif_tif_and_non_image() {
        for p in ["a.gif", "b.tif", "c.tiff", "d.pdf", "e.mp4", "f.docx"] {
            assert!(!is_image_annotate_extension(&PathBuf::from(p)), "{p}");
        }
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app preview::router::tests::image_annotate_extension -- --nocapture`
Expected: FAIL(函数不存在,编译错误)。

- [ ] **Step 3: 实现判定函数**

紧跟在 `is_known_media_extension` 函数定义之后(`router.rs` 里 `pub fn is_known_media_extension` 结束的 `}` 之后)新增:

```rust
/// 走新 `dozer://image-annotate/`(OpenSeadragon + Annotorious)查看器的图片
/// 扩展名子集——仅浏览器引擎(WebKit/wry)能原生解码的光栅格式。`gif`(动画会
/// 丢)与 `tif`/`tiff`(WebKit 不能原生解码,Flyfish 内部自带解码器)刻意排除
/// 在外,继续走 `dozer://flyfish/`(见 2026-09-29 设计稿 §2)。
pub(crate) fn is_image_annotate_extension(path: &Path) -> bool {
    matches!(
        json_extension(path).as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "ico"
    )
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app preview::router::tests::image_annotate_extension -- --nocapture`
Expected: PASS。

- [ ] **Step 5: 跑 router.rs 全部测试确认无回归**

Run: `cargo test -p dozer-app preview::router:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/preview/router.rs
git commit -m "feat(image-annotate): add extension predicate for new viewer subset"
```

---

### Task 5: URL 构造 + Rendered host 绑定分流

**Files:**
- Modify: `crates/dozer-app/src/preview/webview.rs`
- Modify: `crates/dozer-app/src/app/app.rs:3384-3395`(query 参数追加条件)
- Test: `crates/dozer-app/src/preview/webview.rs`(同文件 `#[cfg(test)] mod tests`)、`crates/dozer-app/src/preview/view.rs`(既有 `preview_url_dispatches_html_to_isolated_host_and_others_to_flyfish` 测试所在文件,追加一条新测试)

**Interfaces:**
- Consumes: `is_image_annotate_extension`(Task 4)、`encode_component`/`scheme_query_value`(已存在)。
- Produces:
  - `pub(crate) fn image_annotate_url(path: &std::path::Path) -> String`
  - `pub(crate) fn hosts_rendered_binding(url: &str) -> bool`(供 `app.rs` 与 `flyfish_binding_from_url` 共用的三选一前缀判定)
  - `preview_url` 新增分流分支
  - `flyfish_binding_from_url` 现在也能解析 `dozer://image-annotate/` URL 的绑定

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/preview/webview.rs` 的 `#[cfg(test)] mod tests` 里新增:

```rust
    #[test]
    fn image_annotate_url_carries_path_and_theme() {
        let u = image_annotate_url(std::path::Path::new("/tmp/a b.png"));
        assert!(u.starts_with("dozer://image-annotate/host.html?p="));
        assert!(u.contains("theme="));
    }

    #[test]
    fn hosts_rendered_binding_covers_all_three_prefixes() {
        assert!(hosts_rendered_binding("dozer://flyfish/host.html?p=x"));
        assert!(hosts_rendered_binding("dozer://html/host.html?p=x"));
        assert!(hosts_rendered_binding("dozer://image-annotate/host.html?p=x"));
        assert!(!hosts_rendered_binding("dozer://editor/index.html"));
    }

    #[test]
    fn image_annotate_binding_parses_query() {
        let q = "?p=x&proj=3&panel=files&tab=7&doc=p3-t7";
        let binding =
            flyfish_binding_from_url(&format!("dozer://image-annotate/host.html{q}"));
        assert_eq!(binding.map(|b| b.tab_id), Some(7));
    }
```

追加到 `crates/dozer-app/src/preview/view.rs` 现有 `preview_url_dispatches_html_to_isolated_host_and_others_to_flyfish` 测试旁边:

```rust
    #[test]
    fn preview_url_dispatches_browser_native_images_to_annotate_host() {
        for p in ["/tmp/a.png", "/tmp/b.jpg", "/tmp/c.jpeg", "/tmp/d.webp", "/tmp/e.bmp", "/tmp/f.ico"] {
            let u = preview_url(std::path::Path::new(p));
            assert!(u.starts_with("dozer://image-annotate/host.html?"), "{p} -> {u}");
        }
        for p in ["/tmp/g.gif", "/tmp/h.tif", "/tmp/i.tiff", "/tmp/j.pdf"] {
            let u = preview_url(std::path::Path::new(p));
            assert!(u.starts_with("dozer://flyfish/host.html?"), "{p} -> {u}");
        }
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app preview::webview::tests::image_annotate preview::webview::tests::hosts_rendered_binding preview::view::tests::preview_url_dispatches_browser_native_images -- --nocapture`
Expected: FAIL(编译错误,函数不存在;或 `preview_url` 分支缺失导致断言失败)。

- [ ] **Step 3: 实现 `image_annotate_url` + `hosts_rendered_binding`,接线 `preview_url`/`flyfish_binding_from_url`**

在 `crates/dozer-app/src/preview/webview.rs` 里,`html_url` 函数之后新增:

```rust
/// 图片标注 host 的 URL:png/jpg/jpeg/webp/bmp/ico 专用(见
/// `is_image_annotate_extension`),不带 `&ln=1`/`&fs=` 这类文本预览参数
/// (该 host 不渲染文本)。
pub(crate) fn image_annotate_url(path: &std::path::Path) -> String {
    format!(
        "dozer://image-annotate/host.html?p={}&theme={}",
        encode_component(&path.to_string_lossy()),
        scheme_query_value()
    )
}

/// 是否是需要 T9 envelope 绑定(proj/panel/tab/doc)的 Rendered host URL——
/// Flyfish / 隔离 HTML / 图片标注 三者之一。`app.rs` 据此决定要不要往 URL
/// 追加绑定查询串,`flyfish_binding_from_url` 据此决定要不要解析,避免两处
/// 各写一份前缀列表、迟早漏改其中一处。
pub(crate) fn hosts_rendered_binding(url: &str) -> bool {
    url.starts_with("dozer://flyfish/")
        || url.starts_with("dozer://html/")
        || url.starts_with("dozer://image-annotate/")
}
```

把 `flyfish_binding_from_url` 开头的守卫换成复用这个新函数:

```rust
pub(crate) fn flyfish_binding_from_url(url: &str) -> Option<HostBinding> {
    if !hosts_rendered_binding(url) {
        return None;
    }
    ...
```

（函数体其余部分不变。）

把 `preview_url` 改成三路分派:

```rust
pub(crate) fn preview_url(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => html_url(path),
        _ if is_image_annotate_extension(path) => image_annotate_url(path),
        _ => flyfish_url(path),
    }
}
```

- [ ] **Step 4: 更新 `app.rs` 的绑定查询串追加条件**

`crates/dozer-app/src/app/app.rs:3384-3395` 原代码:

```rust
                for s in &mut specs {
                    if s.url.starts_with("dozer://flyfish/") || s.url.starts_with("dozer://html/") {
                        let doc = format!("p{}-t{}", project.id, s.id);
                        s.url.push_str(&format!(
                            "&proj={}&panel={}&tab={}&doc={}",
                            project.id,
                            crate::preview::panel_token(kind),
                            s.id,
                            crate::preview::encode_component(&doc),
                        ));
                    }
                }
```

改成:

```rust
                for s in &mut specs {
                    if crate::preview::hosts_rendered_binding(&s.url) {
                        let doc = format!("p{}-t{}", project.id, s.id);
                        s.url.push_str(&format!(
                            "&proj={}&panel={}&tab={}&doc={}",
                            project.id,
                            crate::preview::panel_token(kind),
                            s.id,
                            crate::preview::encode_component(&doc),
                        ));
                    }
                }
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p dozer-app preview::webview:: preview::view::tests::preview_url_dispatches -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 6: 全量编译确认 `app.rs` 改动不破坏其它调用点**

Run: `cargo build -p dozer-app`
Expected: 编译成功,无新增警告/错误。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/preview/webview.rs crates/dozer-app/src/preview/view.rs crates/dozer-app/src/app/app.rs
git commit -m "feat(image-annotate): route browser-native images to new viewer URL"
```

---

### Task 6: `ImageAnnotateEvent` 事件类型 + 解析器

**Files:**
- Modify: `crates/dozer-app/src/preview/webview_protocol.rs`
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub enum ImageAnnotateEvent { Ready, DocumentLoaded { revision: u64, bytes: u64, error: Option<String> }, Failed { message: String, recoverable: bool }, AnnotationsChanged { annotations: Vec<serde_json::Value> } }`
  - `pub fn parse_image_annotate_event(raw: &str) -> Result<WebviewEnvelope<ImageAnnotateEvent>, ProtocolError>`

- [ ] **Step 1: 写失败测试**

追加到 `crates/dozer-app/src/preview/webview_protocol.rs` 的 `#[cfg(test)] mod tests`(复用文件里已有的 `binding()`/`raw()` helper):

```rust
    /// Task 6:image-annotate envelope 解析 + 归属校验,镜像
    /// `parses_and_validates_flyfish_events`。
    #[test]
    fn parses_and_validates_image_annotate_events() {
        let raw_str = r#"{"protocol_version":1,"project_id":7,"panel":"files","tab_id":3,"document_id":"p7-t3","revision":0,"request_id":null,"payload":{"kind":"failed","message":"boom","recoverable":true}}"#;
        let env = parse_image_annotate_event(raw_str).unwrap();
        assert_eq!(
            env.payload,
            ImageAnnotateEvent::Failed {
                message: "boom".into(),
                recoverable: true
            }
        );
        let good = HostBinding::new(7, PanelKind::Files, 3, "p7-t3".into());
        assert!(env.validate(&good).is_ok());
        assert!(
            env.validate(&HostBinding::new(8, PanelKind::Files, 3, "p7-t3".into()))
                .is_err()
        );

        for (payload_raw, want) in [
            (r#"{"kind":"ready"}"#, ImageAnnotateEvent::Ready),
            (
                r#"{"kind":"document_loaded"}"#,
                ImageAnnotateEvent::DocumentLoaded {
                    revision: 0,
                    bytes: 0,
                    error: None,
                },
            ),
            (
                r#"{"kind":"document_loaded","error":"解码失败"}"#,
                ImageAnnotateEvent::DocumentLoaded {
                    revision: 0,
                    bytes: 0,
                    error: Some("解码失败".into()),
                },
            ),
            (
                r#"{"kind":"annotations_changed","annotations":[{"id":"a1","body":[]}]}"#,
                ImageAnnotateEvent::AnnotationsChanged {
                    annotations: vec![serde_json::json!({"id": "a1", "body": []})],
                },
            ),
        ] {
            let full = format!(
                r#"{{"protocol_version":1,"project_id":1,"panel":"files","tab_id":1,"document_id":"d","payload":{payload_raw}}}"#
            );
            assert_eq!(parse_image_annotate_event(&full).unwrap().payload, want);
        }

        let bad = r#"{"protocol_version":1,"payload":{"kind":"nope"}}"#;
        assert!(matches!(
            parse_image_annotate_event(bad),
            Err(ProtocolError::UnknownPayload(_))
        ));
        assert!(parse_image_annotate_event("not json").is_err());
    }

    #[test]
    fn rejects_oversized_image_annotate_message() {
        let big = "a".repeat(MAX_MESSAGE_BYTES + 1);
        assert!(matches!(
            parse_image_annotate_event(&big),
            Err(ProtocolError::TooLarge { .. })
        ));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app preview::webview_protocol::tests::parses_and_validates_image_annotate_events preview::webview_protocol::tests::rejects_oversized_image_annotate_message -- --nocapture`
Expected: FAIL(编译错误,类型/函数不存在)。

- [ ] **Step 3: 实现类型与解析器**

紧跟在 `parse_flyfish_event` 函数结束的 `}` 之后新增:

```rust
/// 图片标注 host(annotorious + OpenSeadragon)的事件。与 `FlyfishEvent`
/// 平行但不复用——多一个标注特有的 `AnnotationsChanged`,且这个 host 没有
/// Flyfish 那样的文档内搜索/自定义标题,不需要 `SearchState`/`Title`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageAnnotateEvent {
    /// host 脚本初始化完成(OpenSeadragon viewer 已挂载)。**不代表图片已解码
    /// 完成**;仅用于清除错误并保持 loading,等 `DocumentLoaded` 才结束加载。
    Ready,
    /// 图片已完成解码并首帧绘制(OpenSeadragon `open` 事件)。`revision`/
    /// `bytes` 恒 0(host 不统计,仅为与 Flyfish/editor 事件形态对齐,便于
    /// 复用 update.rs 既有处理惯例)。
    DocumentLoaded {
        #[serde(default)]
        revision: u64,
        #[serde(default)]
        bytes: u64,
        #[serde(default)]
        error: Option<String>,
    },
    /// 渲染失败(图片解码失败、`open-failed` 等)。
    Failed { message: String, recoverable: bool },
    /// 标注增删改后,当前全部标注的完整快照。每条标注是 annotorious 原生的
    /// W3C Web Annotation 风格 JSON;本轮 Rust 侧**不解析内部结构**,只当
    /// 不透明值存取——持久化/发给 agent 的格式留到后续设计,现在定成强类型
    /// 反而会绑死一个仓促格式(见 2026-09-29 设计稿 §5)。
    AnnotationsChanged {
        annotations: Vec<serde_json::Value>,
    },
}

/// 解析一条图片标注 host 事件。与 [`parse_flyfish_event`] 同规则(超大/非法/
/// 未知不 panic)。
pub fn parse_image_annotate_event(
    raw: &str,
) -> Result<WebviewEnvelope<ImageAnnotateEvent>, ProtocolError> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::TooLarge { bytes: raw.len() });
    }
    let env: WebviewEnvelope<serde_json::Value> =
        serde_json::from_str(raw).map_err(|e| ProtocolError::BadJson(e.to_string()))?;
    let kind = env
        .payload
        .get("kind")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let payload: ImageAnnotateEvent = serde_json::from_value(env.payload)
        .map_err(|_| ProtocolError::UnknownPayload(kind.clone()))?;
    Ok(WebviewEnvelope {
        protocol_version: env.protocol_version,
        project_id: env.project_id,
        panel: env.panel,
        tab_id: env.tab_id,
        document_id: env.document_id,
        revision: env.revision,
        request_id: env.request_id,
        payload,
    })
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app preview::webview_protocol::tests::parses_and_validates_image_annotate_events preview::webview_protocol::tests::rejects_oversized_image_annotate_message -- --nocapture`
Expected: PASS。

- [ ] **Step 5: 跑 webview_protocol.rs 全部测试确认无回归**

Run: `cargo test -p dozer-app preview::webview_protocol:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/preview/webview_protocol.rs
git commit -m "feat(image-annotate): add ImageAnnotateEvent type and parser"
```

---

### Task 7: `PreviewTab.image_annotations` 字段 + `PreviewPane::apply_image_annotate_event`

**Files:**
- Modify: `crates/dozer-app/src/preview/state.rs`(字段定义)
- Modify: `crates/dozer-app/src/preview/view.rs`(3 处 `PreviewTab` 字面量初始化 + 新方法)
- Test: `crates/dozer-app/src/preview/view.rs`(同文件 `#[cfg(test)] mod tests`,或紧邻 `impl PreviewPane` 新建的测试模块——沿用文件里既有测试组织方式)

**Interfaces:**
- Consumes: `ImageAnnotateEvent`(Task 6)、`PreviewRuntime::None`/`BackendState::{Ready,Failed}`/`PreviewError::new`(已存在)。
- Produces: `PreviewTab.image_annotations: Vec<serde_json::Value>`、`PreviewPane::apply_image_annotate_event(&mut self, tab_id: usize, event: ImageAnnotateEvent)`(供 Task 8 的 `update.rs` 胶水调用)。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/preview/view.rs` 靠近其它 `PreviewPane` 相关测试的 `#[cfg(test)] mod tests` 里新增(若该文件测试模块尚未 `use crate::preview::webview_protocol::ImageAnnotateEvent;` 之类的导入,按现有 `use super::*;` 惯例应已经可见):

```rust
    fn image_tab(id: usize, backend_state: BackendState) -> PreviewTab {
        let mut tab = placeholder_tab(id);
        tab.kind = TabKind::File(PathBuf::from("/tmp/photo.png"));
        tab.backend_state = backend_state;
        tab.load_state = PreviewLoadState::starting(1, PreviewLoadStage::Profiling);
        tab
    }

    #[test]
    fn apply_image_annotate_event_ready_clears_web_error() {
        let mut pane = PreviewPane::default();
        let mut tab = image_tab(1, BackendState::Loading);
        tab.web_error = Some("旧错误".into());
        pane.tabs.push(tab);
        pane.apply_image_annotate_event(1, ImageAnnotateEvent::Ready);
        assert_eq!(pane.tabs()[1].web_error, None);
    }

    #[test]
    fn apply_image_annotate_event_document_loaded_transitions_ready() {
        let mut pane = PreviewPane::default();
        pane.tabs.push(image_tab(1, BackendState::Loading));
        pane.apply_image_annotate_event(
            1,
            ImageAnnotateEvent::DocumentLoaded {
                revision: 0,
                bytes: 0,
                error: None,
            },
        );
        assert_eq!(pane.tabs()[1].backend_state, BackendState::Ready);
        assert!(!pane.tabs()[1].load_state.is_active());
    }

    #[test]
    fn apply_image_annotate_event_document_loaded_with_error_transitions_failed() {
        let mut pane = PreviewPane::default();
        pane.tabs.push(image_tab(1, BackendState::Loading));
        pane.apply_image_annotate_event(
            1,
            ImageAnnotateEvent::DocumentLoaded {
                revision: 0,
                bytes: 0,
                error: Some("解码失败".into()),
            },
        );
        assert!(pane.tabs()[1].backend_state.is_failed());
        assert_eq!(pane.tabs()[1].web_error.as_deref(), Some("解码失败"));
    }

    #[test]
    fn apply_image_annotate_event_failed_transitions_failed_state() {
        let mut pane = PreviewPane::default();
        pane.tabs.push(image_tab(1, BackendState::Loading));
        pane.apply_image_annotate_event(
            1,
            ImageAnnotateEvent::Failed {
                message: "boom".into(),
                recoverable: true,
            },
        );
        assert!(pane.tabs()[1].backend_state.is_failed());
    }

    #[test]
    fn apply_image_annotate_event_annotations_changed_stores_snapshot() {
        let mut pane = PreviewPane::default();
        pane.tabs.push(image_tab(1, BackendState::Ready));
        let annotations = vec![serde_json::json!({"id": "a1"})];
        pane.apply_image_annotate_event(
            1,
            ImageAnnotateEvent::AnnotationsChanged {
                annotations: annotations.clone(),
            },
        );
        assert_eq!(pane.tabs()[1].image_annotations, annotations);
    }

    #[test]
    fn apply_image_annotate_event_ignores_unknown_tab_id() {
        let mut pane = PreviewPane::default();
        pane.tabs.push(image_tab(1, BackendState::Ready));
        // tab_id 99 不存在——必须静默返回,不能 panic。
        pane.apply_image_annotate_event(99, ImageAnnotateEvent::Ready);
    }

    #[test]
    fn apply_image_annotate_event_stale_document_loaded_does_not_finish_new_load() {
        let mut pane = PreviewPane::default();
        let mut tab = image_tab(1, BackendState::Ready);
        tab.load_state = PreviewLoadState::default(); // 非 active(已完成)
        pane.tabs.push(tab);
        pane.apply_image_annotate_event(
            1,
            ImageAnnotateEvent::DocumentLoaded {
                revision: 0,
                bytes: 0,
                error: None,
            },
        );
        // 迟到 ACK 不应改变已经是 Ready 且非 active 的状态。
        assert_eq!(pane.tabs()[1].backend_state, BackendState::Ready);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app preview::view::tests::apply_image_annotate_event -- --nocapture`
Expected: FAIL(编译错误:`image_annotations` 字段与 `apply_image_annotate_event` 方法都不存在)。

- [ ] **Step 3: 加字段**

`crates/dozer-app/src/preview/state.rs`,在 `PreviewTab` 结构体的 `task_cancel` 字段之前新增:

```rust
    /// 图片标注 host(annotorious)当前的全部标注快照,每条是不透明
    /// `serde_json::Value`(annotorious 原生格式)。本轮不落盘、不发给
    /// agent,只随 tab 生命周期存在(见 2026-09-29 设计稿 §5)。
    pub image_annotations: Vec<serde_json::Value>,
```

- [ ] **Step 4: 更新 3 处字面量初始化**

`crates/dozer-app/src/preview/view.rs` 里的 `placeholder_tab`(约第 24 行起)、`push_tab` 内的 `let tab = PreviewTab { ... }`(约第 549 行起)、`push_shell_tab` 内的 `let tab = PreviewTab { ... }`(约第 638 行起)——三处都在 `tabular_host_ready: false,` 之后、`task_cancel: ...` 之前加一行:

```rust
            image_annotations: Vec::new(),
```

- [ ] **Step 5: 实现 `apply_image_annotate_event`**

在 `crates/dozer-app/src/preview/view.rs` 的 `impl PreviewPane` 块里(紧跟 `tabs_mut` 方法之后)新增:

```rust
    /// 图片标注 host 事件应用。镜像 `Message::FlyfishEvent` 在 `update.rs`
    /// 里的处理逻辑,但提取成纯函数——`update.rs` 只做 `with_project` 找到
    /// 对应 pane 的胶水,真正的状态机在这里,可以脱离整个 `App` 单测。
    pub fn apply_image_annotate_event(&mut self, tab_id: usize, event: ImageAnnotateEvent) {
        let Some(tab) = self.tabs_mut().iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        match event {
            ImageAnnotateEvent::Ready => {
                tab.web_error = None;
            }
            ImageAnnotateEvent::DocumentLoaded { error, .. } => {
                if !tab.load_state.is_active() {
                    return;
                }
                if let Some(message) = error {
                    tab.runtime = PreviewRuntime::None;
                    tab.web_error = Some(message.clone());
                    let _ = tab
                        .backend_state
                        .try_transition(BackendState::Failed(PreviewError::new(message, true)));
                    tab.load_state.finish();
                } else {
                    tab.web_error = None;
                    let _ = tab.backend_state.try_transition(BackendState::Ready);
                    tab.load_state.finish();
                }
            }
            ImageAnnotateEvent::Failed {
                message,
                recoverable,
            } => {
                tab.runtime = PreviewRuntime::None;
                tab.web_error = Some(message.clone());
                let _ = tab.backend_state.try_transition(BackendState::Failed(
                    PreviewError::new(message, recoverable),
                ));
                if tab.load_state.is_active() {
                    tab.load_state.finish();
                }
            }
            ImageAnnotateEvent::AnnotationsChanged { annotations } => {
                tab.image_annotations = annotations;
            }
        }
    }
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p dozer-app preview::view::tests::apply_image_annotate_event -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 7: 跑 preview 全部测试确认无回归(3 处字面量改动影响面广)**

Run: `cargo test -p dozer-app preview:: -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/preview/state.rs crates/dozer-app/src/preview/view.rs
git commit -m "feat(image-annotate): add annotation snapshot field and event state machine"
```

---

### Task 8: `Message::ImageAnnotateEvent` + IPC 派发接线

**Files:**
- Modify: `crates/dozer-app/src/app/message.rs`
- Modify: `crates/dozer-app/src/runtime.rs`
- Modify: `crates/dozer-app/src/app/update.rs`

**Interfaces:**
- Consumes: `ImageAnnotateEvent`/`parse_image_annotate_event`(Task 6)、`PreviewPane::apply_image_annotate_event`(Task 7)、`hosts_rendered_binding`/`flyfish_binding_from_url`(Task 5)。
- Produces: `Message::ImageAnnotateEvent(HostBinding, WebviewEnvelope<ImageAnnotateEvent>)` 端到端可用(webview IPC → Rust 状态机)。

这一层是纯接线,和 `Message::FlyfishEvent` 的既有接线一样,仓库里没有对 `runtime.rs` 的 wry IPC 闭包做独立单测(闭包捕获 `wry`/`tao` 运行时句柄,现有 Flyfish/JSON/Tabular 接线同样没有);正确性由 Task 6(解析器)、Task 7(状态机)的单测 + 人工验收清单(Task 9)共同兜底。

- [ ] **Step 1: 加 `Message` 变体**

`crates/dozer-app/src/app/message.rs`,紧跟 `FlyfishEvent(...)` 变体之后新增:

```rust
    /// 图片标注 host(annotorious + OpenSeadragon)发回的已校验事件
    /// (ready/document_loaded/failed/annotations_changed)。binding 由 Rust
    /// 从 webview URL 解析,不采信 JS 自报归属。
    ImageAnnotateEvent(
        crate::preview::HostBinding,
        crate::preview::WebviewEnvelope<crate::preview::ImageAnnotateEvent>,
    ),
```

- [ ] **Step 2: `runtime.rs` 加 host 类型判定 + 派发分支**

`crates/dozer-app/src/runtime.rs` 里,`let is_flyfish_host = spec.url.starts_with("dozer://flyfish/");`(约第 377 行)之后新增:

```rust
                // image-annotate host 与 Flyfish/HTML 共用 `flyfish_binding`
                // (`hosts_rendered_binding` 三选一),这里单独判定用来在 catch-all
                // 分支里选对解析器/Message 变体。
                let is_image_annotate_host = spec.url.starts_with("dozer://image-annotate/");
```

把 catch-all 分支(约第 506-525 行,`_ => { let looks_like_envelope = ...`)里原本的:

```rust
                                if let Some(binding) = flyfish_binding.as_ref()
                                    && looks_like_envelope
                                {
                                    match crate::preview::parse_flyfish_event(body) {
```

改成先判 image-annotate、再判 flyfish:

```rust
                                if let Some(binding) = flyfish_binding.as_ref()
                                    && looks_like_envelope
                                    && is_image_annotate_host
                                {
                                    match crate::preview::parse_image_annotate_event(body) {
                                        Ok(event) => {
                                            if let Err(error) = event.validate(binding) {
                                                tracing::warn!(%error, "拒绝无效 image-annotate IPC");
                                            } else {
                                                let _ = ipc_proxy.send_event(
                                                    Message::ImageAnnotateEvent(
                                                        binding.clone(),
                                                        event,
                                                    ),
                                                );
                                            }
                                        }
                                        Err(error) => {
                                            tracing::warn!(%error, "无法解析 image-annotate IPC");
                                        }
                                    }
                                } else if let Some(binding) = flyfish_binding.as_ref()
                                    && looks_like_envelope
                                {
                                    match crate::preview::parse_flyfish_event(body) {
```

（原来的 `parse_flyfish_event` 分支体、以及后面的 `else if webview_id == ...` 分支全部保持不变,只是从 `if` 换成 `else if`。）

- [ ] **Step 3: `update.rs` 加 Message 处理**

`crates/dozer-app/src/app/update.rs`,紧跟 `Message::FlyfishEvent(binding, event) => { ... }` 整个 match arm(约第 797-891 行)结束的 `}` 之后新增:

```rust
            Message::ImageAnnotateEvent(binding, event) => {
                self.with_project(binding.project_id, move |ws, _io| {
                    let pane = if binding.panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    pane.apply_image_annotate_event(binding.tab_id, event.payload);
                });
            }
```

- [ ] **Step 4: 全量编译确认接线正确**

Run: `cargo build -p dozer-app`
Expected: 编译成功,无新增警告(尤其确认 `Message::ImageAnnotateEvent` 两处使用——`runtime.rs` 发送、`update.rs` 接收——类型完全匹配,没有 match 分支遗漏警告)。

- [ ] **Step 5: 跑 dozer-app 全部单测确认无回归**

Run: `cargo test -p dozer-app`
Expected: 全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/app/message.rs crates/dozer-app/src/runtime.rs crates/dozer-app/src/app/update.rs
git commit -m "feat(image-annotate): wire IPC dispatch for ImageAnnotateEvent"
```

---

### Task 9: 全量校验 + 人工验收清单

**Files:** 无新增/修改代码文件(除非验证中发现问题需要回头小修)。

- [ ] **Step 1: 全 workspace 编译 + lint + 格式检查**

```bash
cargo build
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全部无错误(`cargo fmt --check` 若报格式问题,跑 `cargo fmt` 后重新 `git add`/`git commit`)。

- [ ] **Step 2: 全 workspace 测试**

```bash
cargo test
```

Expected: 全部 PASS,尤其确认 Task 1-8 新增的所有测试(vendor 存在性、协议路由、扩展名判定、URL 分流、事件解析、状态机)都在这次全量跑里出现且通过。

- [ ] **Step 3: 启动 GUI,跑一遍 spec §7 的人工验收清单**

```bash
cargo run -p dozer-app
```

对照 `docs/superpowers/specs/2026-09-29-image-annotate-viewer-design.md` §7 逐项打勾:
- [ ] 打开 png/jpg/jpeg/webp/bmp/ico 各一张,确认走新查看器且能正常显示
- [ ] 打开 gif/tif/tiff 各一张,确认仍走 Flyfish(gif 动画不丢)
- [ ] 缩放(滚轮 + 导航控件按钮)、平移(拖拽)正常
- [ ] 画矩形 / 点 / 多边形标注,能加文字备注
- [ ] 关闭 tab 后重新打开同一图片,确认标注已清空(符合"不持久化"设计)
- [ ] 深色/浅色主题切换,查看器与标注高亮色跟随
- [ ] 故意打开一个损坏的图片文件,确认走 `Failed` → fallback 页而不是白屏或卡死

任何一项不通过,回到对应 Task 定位问题、修复、重新提交,不要在验收清单打勾造假。

- [ ] **Step 4: 最终 Commit(若 Step 1-3 期间有修复性改动)**

```bash
git add -A
git status  # 确认只包含预期文件,没有意外改动
git commit -m "fix(image-annotate): address issues found in final verification"
```

（若 Step 1-3 全程无需修复,本步骤跳过——不要为了"有个 commit"硬造一次空提交。）
