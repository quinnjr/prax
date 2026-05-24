// Fixture: aggregate! with _sum applied to a String column (email)
// Expected diagnostic: "is not numeric; `_sum` requires a numeric column"

fn main() {
    let client = ();
    let _ = prax_codegen::aggregate!(client.user, {
        _sum: { email: true },
    });
}
