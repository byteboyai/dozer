//! `install` 是进程级单例:同值重复安装必须幂等成功。
//! 独立集成测试文件 = 独立进程,与其它 install 测试互不污染。
use dozer_core::paths::{ProductDirs, ProductId, install, product};

fn digger(root: &std::path::Path) -> ProductId {
    ProductId::custom(
        "digger",
        "DIGGER_SESSION_ID",
        "digger-hook",
        "diggerd.sock",
        "digger.db",
        ProductDirs::Fixed {
            config: root.join("config"),
            state: root.join("state"),
        },
    )
}

#[test]
fn install_twice_with_the_same_product_is_ok() {
    let root = tempfile::tempdir().unwrap();
    let first = digger(root.path());
    let second = digger(root.path());

    assert!(install(first).is_ok(), "首次安装自定义产品必须成功");
    assert!(install(second).is_ok(), "同值重复安装必须幂等返回 Ok");
    assert_eq!(product().name, "digger", "进程身份应为先安装的产品");
}
