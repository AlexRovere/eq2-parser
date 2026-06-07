fn main() {
    // Icône de l'exécutable Windows (Explorateur, barre des tâches).
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            // Pas bloquant : l'app fonctionne sans icône embarquée.
            println!("cargo:warning=icône non embarquée : {e}");
        }
    }
}
