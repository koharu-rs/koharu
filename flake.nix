{
  description = "Development environment for Koharu, an ML-powered manga translator";

  # nixos-unstable snapshot (Nov 2026). Pinned by revision so the LLVM and
  # toolchain set below stays stable; run `nix flake update` to move forward.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/d4f25a3e562008648d45bcf2d70bcc7422cad84e";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };

          # Toolchain versions from packages/docs/development/setup.mdx.
          # The repository does not pin Rust; CI uses rustup, so the toolchain
          # comes from rustup rather than nixpkgs to keep the wasm32 target
          # and cargo subcommands (wasm-pack, cargo-tauri) behaving the same
          # as upstream.
          rustChannel = "1.97.1";

          # LLVM 22 supplies the libclang that bindgen needs in the
          # koharu-{llama,diffusion,torch}-sys crates.
          llvm = pkgs.llvmPackages_22;
          libclang = llvm.libclang.lib or llvm.libclang;

          # CJK fonts for the typesetting tests (CI installs fonts-noto-cjk),
          # layered on top of the host system font configuration so the editor
          # and tests still see every other installed font.
          fontsConf = pkgs.writeText "koharu-fonts.conf" ''
            <?xml version="1.0"?>
            <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
            <fontconfig>
              <include ignore_missing="yes">/etc/fonts/fonts.conf</include>
              <dir>${pkgs.noto-fonts-cjk-sans}</dir>
              <cachedir>~/.cache/fontconfig</cachedir>
            </fontconfig>
          '';

          # System libraries libcef.so and its helper processes load at
          # runtime. The Ubuntu runners CI builds on carry these in the base
          # image; on NixOS they must come from the store. If the app reports
          # a missing library at startup, add it here.
          runtimeLibs = with pkgs; [
            nss
            nspr
            atk
            at-spi2-atk
            at-spi2-core
            cups
            dbus
            dbus-glib
            glib
            cairo
            pango
            libgbm
            udev # libudev.so.1, required by libdrm and the CEF helpers
            libdrm
            gtk3
            alsa-lib
            libxkbcommon
            mesa
            libglvnd # libEGL.so.1/libGL.so.1 dispatchers; CEF's GPU process dlopens them and resolves the NVIDIA vendor libs through /run/opengl-driver
            vulkan-loader # libvulkan.so.1; finds the NVIDIA ICD via /run/opengl-driver/share (XDG_DATA_DIRS)
            expat
            xdotool # libxdo.so for Tauri's menu integration
            libx11
            libxcb
            libxcomposite
            libxcursor
            libxdamage
            libxext
            libxfixes
            libxi
            libxrandr
            libxrender
            libxscrnsaver
          ];

          gtkSchemas = pkgs.lib.concatStringsSep ":" [
            "${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}"
            "${pkgs.gtk4}/share/gsettings-schemas/${pkgs.gtk4.name}"
          ];
        in
        {
          default = pkgs.mkShell {
            name = "koharu";

            packages = [
              pkgs.rustup # toolchain proxy; installs the pinned channel below
              pkgs.bun # >= 1.3.14
              pkgs.nodejs # packages/bridge build scripts invoke `npm prefix`
              pkgs.git # cargo dependencies pulled from git revisions
              pkgs.cmake # cef-dll-sys builds its CEF glue with cmake
              pkgs.file # used while unpacking the CEF archive
              pkgs.pkg-config
              pkgs.dbus # headless tests (CI installs dbus-x11)
              pkgs.gnome-keyring # headless tests
            ];

            buildInputs = [
              # The library set CI installs on Ubuntu: libgtk-4-dev,
              # libxdo-dev (shipped by xdotool), libssl-dev,
              # libayatana-appindicator3-dev, librsvg2-dev. gtk3 covers
              # rfd's file dialogs.
              pkgs.gtk4
              pkgs.gtk3
              pkgs.glib
              pkgs.xdotool
              pkgs.openssl
              pkgs.libayatana-appindicator
              pkgs.librsvg
            ];

            # bindgen looks up libclang through this.
            LIBCLANG_PATH = "${libclang}/lib";

            # bindgen parses headers with libclang directly, which has no
            # default system include path on NixOS. Include libstdc++ headers
            # as well for C++ headers such as koharu-torch-sys.
            BINDGEN_EXTRA_CLANG_ARGS = pkgs.lib.concatStringsSep " " [
              "-isystem ${pkgs.stdenv.cc.cc}/include/c++/${pkgs.stdenv.cc.cc.version}"
              "-isystem ${pkgs.stdenv.cc.cc}/include/c++/${pkgs.stdenv.cc.cc.version}/${pkgs.stdenv.hostPlatform.config}"
              "-isystem ${pkgs.stdenv.cc.libc.dev}/include"
            ];

            # Resolve the pinned channel without touching the user's
            # rustup defaults.
            RUSTUP_TOOLCHAIN = rustChannel;

            # CJK test fonts on top of the host font configuration.
            FONTCONFIG_FILE = toString fontsConf;

            # Keep the desktop session's data dirs (themes, portals) and add
            # the GSettings schemas the GTK 4 app reads.
            shellHook = ''
              export XDG_DATA_DIRS="${gtkSchemas}:''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"

              # libcef.so resolves its system libraries at runtime.
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath runtimeLibs}:''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

              # CEF's bundled Vulkan loader does not know NixOS's driver paths.
              # Use the active host drivers, which match the running kernel.
              if [ -d /run/opengl-driver ]; then
                export LD_LIBRARY_PATH="/run/opengl-driver/lib:$LD_LIBRARY_PATH"
                export XDG_DATA_DIRS="/run/opengl-driver/share:$XDG_DATA_DIRS"
              fi

              # cargo-tauri lands here through `bun install` (postinstall).
              export PATH="$HOME/.cargo/bin:$PATH"

              if ! rustup toolchain list | grep -q "^${rustChannel}-"; then
                rustup toolchain install ${rustChannel} --profile minimal \
                  --component clippy,rustfmt,rust-src,rust-analyzer
              fi
              if ! rustup target list --installed | grep -q "^wasm32-unknown-unknown$"; then
                rustup target add wasm32-unknown-unknown
              fi

              # The workspace lockfile and postinstall own JS dependencies and
              # the pinned Tauri CLI; keep shell setup on that same path.
              bun install --frozen-lockfile
            '';
          };
        }
      );
    };
}
