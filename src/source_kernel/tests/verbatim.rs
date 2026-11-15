use super::{metadata, scan};

#[test]
fn a_workspace_root_in_verbatim_form_loses_no_unit() {
    let plain = metadata("precision");
    let mut verbatim = metadata("precision");
    verbatim.workspace_root = format!("\\\\?\\{}", plain.workspace_root).into();

    let (plain, verbatim) = (scan(&plain), scan(&verbatim));
    assert!(plain.walk.files_read > 0, "the fixture was not walked");
    assert_eq!(verbatim.walk.files_read, plain.walk.files_read);
    assert_eq!(verbatim.errors, plain.errors);
    assert!(
        verbatim
            .errors
            .iter()
            .all(|error| error.code != "path-outside-workspace"),
        "{:?}",
        verbatim.errors
    );
    assert_eq!(verbatim.candidates.len(), plain.candidates.len());
}
