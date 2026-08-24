fn main() {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/foxycompressor.ico");
        resource
            .compile()
            .expect("could not embed FoxyCompressor icon");
    }
}
