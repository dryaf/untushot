use glib::{prelude::*, FromVariant};

#[test]
fn variant_string_iterator_security_backport() {
    let values = vec!["", "Änderung", "three", "四", "last"];
    let variant = values.to_variant();
    assert_eq!(
        variant.array_iter_str().unwrap().collect::<Vec<_>>(),
        values
    );
    let mut iter = variant.array_iter_str().unwrap();
    assert_eq!(iter.next(), Some(""));
    assert_eq!(iter.next_back(), Some("last"));
    assert_eq!(iter.nth(1), Some("three"));
    assert_eq!(iter.nth_back(0), Some("四"));
    assert_eq!(iter.next(), None);
    assert_eq!(variant.array_iter_str().unwrap().last(), Some("last"));
    assert_eq!(variant.array_iter_str().unwrap().nth_back(2), Some("three"));
    let empty = Vec::<String>::new().to_variant();
    assert_eq!(empty.array_iter_str().unwrap().next(), None);
}

#[test]
fn anyhow_mutable_context_regression() {
    let mut error = anyhow::anyhow!("inner").context(String::from("before"));
    *error.downcast_mut::<String>().unwrap() = "after".into();
    assert_eq!(error.to_string(), "after");
}

#[derive(Debug, PartialEq, glib::Variant)]
struct Record(String, u32);

#[test]
fn patched_macros_produce_working_code() {
    let record = Record("example".into(), 12);
    assert_eq!(Record::from_variant(&record.to_variant()), Some(record));
    let value = std::rc::Rc::new(23);
    let closure = glib::closure_local!(@strong value => move || *value);
    assert_eq!(closure.invoke::<i32>(&[]), 23);
}

#[test]
fn patched_macros_report_invalid_inputs_without_panicking() {
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let artifact = |name: &str| {
        std::fs::read_dir(&deps)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("lib{name}-"))
                    && e.path().extension().is_some_and(|x| x == "rlib")
            })
            .max_by_key(|e| e.metadata().unwrap().modified().unwrap())
            .unwrap()
            .path()
    };
    let glib = artifact("glib");
    let gtk = artifact("gtk");
    let directory = tempfile::tempdir().unwrap();
    for (i, (source, message)) in [
        (
            "#[derive(gtk::CompositeTemplate)] struct Bad;",
            "requires #[template(...)]",
        ),
        (
            "#[derive(glib::Boxed)] struct Bad;",
            "requires #[boxed_type",
        ),
        (
            "#[derive(glib::Variant)] #[variant_enum(unknown)] enum Bad { A }",
            "unknown type",
        ),
        (
            "fn bad() { let _ = glib::closure_local!(@mystery value => move || 1); }",
            "Unknown keyword",
        ),
        (
            "#[glib::object_subclass] struct Bad;",
            "This macro should be used",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let path = directory.path().join(format!("invalid_{i}.rs"));
        std::fs::write(&path, source).unwrap();
        let output = std::process::Command::new("rustc")
            .args([
                "--edition=2021",
                "--crate-type=lib",
                "--emit=metadata",
                "--extern",
            ])
            .arg(format!("glib={}", glib.display()))
            .arg("--extern")
            .arg(format!("gtk={}", gtk.display()))
            .arg("-L")
            .arg(format!("dependency={}", deps.display()))
            .arg("--out-dir")
            .arg(directory.path())
            .arg(&path)
            .output()
            .unwrap();
        let errors = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "invalid macro input accepted");
        assert!(errors.contains(message), "{errors}");
        assert!(!errors.contains("proc macro panicked"), "{errors}");
    }
}
