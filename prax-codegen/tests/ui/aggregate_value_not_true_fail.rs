// Fixture: aggregate! with a non-true value inside a _count block
// Expected diagnostic: "must be `true`"

fn main() {
    let client = ();
    let _ = prax_codegen::aggregate!(client.user, {
        _count: { email: false },
    });
}
