// Fixture: aggregate! with an unknown top-level key
// Expected diagnostic: "unknown key `_foo`"

fn main() {
    let client = ();
    let _ = prax_codegen::aggregate!(client.user, {
        _foo: { id: true },
    });
}
