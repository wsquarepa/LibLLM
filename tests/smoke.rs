mod common;

#[test]
fn common_helpers_compile() {
    let dir = common::temp_dir();
    common::create_data_dirs(dir.path());
    let _key = common::test_key(dir.path());

    let session = common::linear_session(vec![
        common::user_msg("hello"),
        common::assistant_msg("hi"),
    ]);
    assert_eq!(session.tree.branch_path().len(), 2);
}
