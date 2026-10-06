use ansible_password_agent::backend::LinuxBackend;

#[test]
fn set_then_get_roundtrip() {
    // In some minimal environments (no session keyring) the test self-skips.
    if linux_keyutils::KeyRing::from_special_id(
        linux_keyutils::KeyRingIdentifier::UserSession,
        true,
    )
    .is_err()
    {
        eprintln!("skipping: no user session keyring available");
        return;
    }
    let secret = format!("test-secret-{}", std::process::id());
    LinuxBackend::set("test", &secret).expect("set");
    let got = LinuxBackend::get("test")
        .expect("get")
        .expect("cached value");
    assert_eq!(got, secret);
    // overwrite path
    LinuxBackend::set("test", "second").expect("set 2");
    assert_eq!(LinuxBackend::get("test").unwrap().unwrap(), "second");
    // missing key is a clean miss
    assert_eq!(LinuxBackend::get("nonexistent").unwrap(), None);
}
