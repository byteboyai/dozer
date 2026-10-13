/**
 * TS 侧共享插值向量测试(spec §6.2 / L06)。
 * Rust 测试 `crates/dozer-i18n/tests/vectors.rs` 读同一向量文件。
 *
 * 运行:node crates/dozer-app/web/shared/i18n/i18n.test.ts
 * (node v24 直接执行 .ts;无需构建。)
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import assert from "node:assert/strict";
import { test } from "node:test";

import { interpolate } from "./i18n.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
// shared/i18n/ -> web/ -> src/(dozer-app) -> crates/dozer-app -> crates -> rust vectors
const VECTORS = resolve(
  HERE,
  "../../../../dozer-i18n/vectors/interpolation.json",
);

interface Case {
  name: string;
  template: string;
  args: Record<string, string>;
  expect: string | null;
}

const data = JSON.parse(readFileSync(VECTORS, "utf8")) as { interpolate: Case[] };

test("interpolation vectors match", () => {
  assert.ok(data.interpolate.length > 0, "向量不应为空");
  for (const c of data.interpolate) {
    let got: string | null;
    try {
      got = interpolate(c.template, c.args);
    } catch {
      got = null;
    }
    assert.equal(
      got,
      c.expect,
      `向量 \`${c.name}\` 不符:template=${JSON.stringify(c.template)}`,
    );
  }
});
