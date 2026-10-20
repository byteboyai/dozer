//! `dozer-hook` 薄入口:全部逻辑在 `dozer_hook::run_main`,
//! 这里只决定"我是谁"。Digger 的 `digger-hook` 用同一函数,
//! 传自己的产品身份。

fn main() -> ! {
    dozer_hook::run_main(dozer_core::paths::ProductId::dozer())
}
