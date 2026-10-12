# usage-content 内容侧 bundle — 许可证与资源版本

## 来源与许可证

`src/` 下的全部内容是本仓库(Dozer 项目)的一手代码,版权归 Dozer 项目所有,
许可证与仓库根 `Cargo.toml` 一致(MIT)。

打包产物 `assets/usage-content/{usage-content.js,usage-content.css,host.html}`
由 `build.mjs` 生成,`esbuild` 配置为 `legalComments: 'none'`,`preact` 以
ESM 形式内联进 IIFE,不额外保留第三方 legal comment。运行期**不含** Node、
不含任何 CDN / 外部网络引用(见 `crates/dozer-app/src/assets.rs` 的
`usage_content_host_has_strict_csp_and_no_external_refs` 测试)。

## 唯一来源

本目录(源)与 `crates/dozer-panel-usage/assets/usage-content/`(产物)是本
bundle 的**唯一**来源。消费方(Dozer 的 `dozer-app`、Digger 的 `desktop`)
一律从本 crate 取用,不各自留一份副本。

## 资源版本 / 更新流程

- 产物版本随 crate 版本 `crates/dozer-panel-usage/Cargo.toml` 的 `version` 走。
- 修改 `src/` 后必须重新构建并提交产物:

  ```sh
  cd crates/dozer-panel-usage/web/usage-content
  npm ci
  npm run typecheck
  npm test
  npm run build   # 写入 ../../assets/usage-content/
  ```

- 忘记重新构建会被 `dozer-app` 的 `usage_content_bundle_assets_are_present`
  测试(校验 host.html / usage-content.js / usage-content.css 齐全且非空)挡下。
