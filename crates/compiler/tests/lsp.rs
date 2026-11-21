//! Language server behaviour that depends on the host's file loader.
use nested_compiler::json::Json;
use nested_compiler::lsp::LanguageServer;
use std::cell::RefCell;
use std::rc::Rc;

fn message(text: &str) -> Json {
    Json::parse(text).unwrap()
}

/// A server whose loader records every URI it is asked for. `files` are
/// what the "disk" holds.
fn server(
    files: &'static [(&'static str, &'static str)],
) -> (LanguageServer, Rc<RefCell<Vec<String>>>) {
    let asked = Rc::new(RefCell::new(Vec::new()));
    let log = asked.clone();
    let mut lsp = LanguageServer::new();
    lsp.loader = Some(Box::new(move |uri: &str| {
        log.borrow_mut().push(uri.to_owned());
        files
            .iter()
            .filter(|(u, _)| *u == uri || (uri.ends_with('/') && u.starts_with(uri)))
            .map(|(u, t)| (u.to_string(), t.as_bytes().to_vec()))
            .collect()
    }));
    (lsp, asked)
}

fn open(lsp: &mut LanguageServer, uri: &str, text: &str) {
    let text = Json::from(text).stringify();
    lsp.handle(message(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"nested","version":1,"text":{text}}}}}}}"#
    )));
}

#[test]
fn a_lone_file_does_not_read_its_directory() {
    let (mut lsp, asked) = server(&[("file:///tmp/other.bin", "x")]);
    open(
        &mut lsp,
        "file:///tmp/typing.nst",
        "var score: u8 = 0;\nfn update() { score += 1; }\n",
    );
    let replies = lsp.handle(message(
        r#"{"jsonrpc":"2.0","id":1,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///tmp/typing.nst"},"position":{"line":0,"character":5}}}"#,
    ));
    assert!(replies[0].stringify().contains("score"));
    let asked = asked.borrow();
    assert!(
        asked.iter().all(|u| u.ends_with("main.nst")),
        "listed a directory: {asked:?}"
    );
}

#[test]
fn a_project_loads_its_modules() {
    let (mut lsp, asked) = server(&[
        (
            "file:///game/main.nst",
            "use util;\nfn update() { util::tick(); }\n",
        ),
        ("file:///game/util.nst", "pub fn tick() {}\n"),
    ]);
    open(
        &mut lsp,
        "file:///game/main.nst",
        "use util;\nfn update() { util::tick(); }\n",
    );
    let replies = lsp.handle(message(
        r#"{"jsonrpc":"2.0","id":1,"method":"textDocument/definition","params":{"textDocument":{"uri":"file:///game/main.nst"},"position":{"line":1,"character":21}}}"#,
    ));
    assert!(
        replies[0].stringify().contains("file:///game/util.nst"),
        "{}",
        replies[0].stringify()
    );
    assert!(asked.borrow().iter().any(|u| u == "file:///game/"));
}
