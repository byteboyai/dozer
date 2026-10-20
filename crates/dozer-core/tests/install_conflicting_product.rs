//! `install` 是进程级单例:装了 A 之后装 B(不同值)必须报错,
//! 且进程身份保持 A——同一进程绝不允许两个产品的状态串线。
//! 独立集成测试文件 = 独立进程。
use dozer_core::paths::{ProductDirs, ProductId, install, product};

#[test]
fn install_twice_with_a_different_product_is_an_error() {
    let root = tempfile::tempdir().unwrap();
    let digger = ProductId::custom(
        "digger",
        "DIGGER_SESSION_ID",
        "digger-hook",
        "diggerd.sock",
        "digger.db",
        ProductDirs::Fixed {
            config: root.path().join("config"),
            state: root.path().join("state"),
        },
    );

    assert!(install(ProductId::dozer()).is_ok(), "默认产品先装");
    let err = install(digger).expect_err("装了 dozer 再装 digger 必须报错");
    assert_eq!(err.installed.name, "dozer", "报错里应带回已安装的产品");
    assert_eq!(err.attempted.name, "digger", "报错里应带回被拒的产品");
    assert_eq!(product().name, "dozer", "进程身份不得被失败的安装改变");
}
