//! Fixture adversariale de référence du modèle de score.
//!
//! Elle rassemble les défauts que le PRD score-credibility-kernel mesure comme
//! coexistant avec une note de 99 sous `core-v1`. Sous `core-v2`, l'injection de
//! commande porte le tier `P0`, donc la note globale ne peut plus dépasser son
//! plafond quelle que soit la moyenne des autres dimensions.
//!
//! Ce que le catalogue courant ne détecte pas encore, secret en dur,
//! concaténation SQL, `unsafe` non justifié, `unwrap`, `panic!` et indexation
//! non vérifiée, reste écrit ici volontairement: la fixture doit rester le point
//! de mesure des tranches suivantes.

/// Détecté: `rust_doctor::source::dynamic_shell_command`, tier `P0`.
pub fn run_user_command(user: &str) -> std::process::Output {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("echo {user}"))
        .output()
        .unwrap_or_else(|error| panic!("the shell should answer: {error}"))
}

/// Détecté: `clippy::unimplemented`, tier `P1`.
pub fn rotate_credentials() -> &'static str {
    unimplemented!()
}

/// Détecté: `clippy::todo`, tier `P2`.
pub fn revoke_session(_token: &str) -> bool {
    todo!()
}

/// Détecté: `clippy::dbg_macro`, tier `P3`.
pub fn trace_request(path: &str) -> usize {
    dbg!(path.len())
}

/// Non détecté par le catalogue courant: identifiant de paiement en dur.
///
/// Le littéral ne reprend volontairement la forme d'aucun fournisseur réel: la
/// protection de push de GitHub refuserait la fixture, et le scan de secrets
/// sur fichiers bruts est explicitement hors périmètre de cette tranche. Ce
/// qu'il documente ici, c'est la présence d'un identifiant en clair dans le
/// code source.
pub const PAYMENT_KEY: &str = "PLACEHOLDER-PAYMENT-CREDENTIAL-DO-NOT-USE";

/// Non détecté par le catalogue courant: concaténation SQL.
pub fn user_query(name: &str) -> String {
    format!("SELECT * FROM users WHERE name = '{name}'")
}

/// Non détecté par le catalogue courant: `unsafe` sans justification.
pub fn first_byte(bytes: &[u8]) -> u8 {
    unsafe { *bytes.get_unchecked(0) }
}

/// Non détecté par le catalogue courant: indexation non vérifiée et `unwrap`.
pub fn third_field(line: &str) -> String {
    let fields: Vec<&str> = line.split(',').collect();
    let parsed: u8 = fields[2].parse().unwrap();
    parsed.to_string()
}
