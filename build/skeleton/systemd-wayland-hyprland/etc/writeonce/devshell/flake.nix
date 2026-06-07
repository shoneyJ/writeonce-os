# /etc/writeonce/devshell/flake.nix — WriteOnce development environment.
#
# A project-scoped, ephemeral dev shell (NOT installed into the user profile —
# that's what Home Manager is for). Enter it with:
#     nix develop /etc/writeonce/devshell
# or copy this flake into a project repo as flake.nix and `nix develop`.
#
# nvim + tmux read the Home-Manager-managed configs from ~/.config, so the
# editors come up fully configured inside the shell. Selection only — no package
# definitions authored (consume nixpkgs as-is). flake.lock is generated on first
# use; commit it for reproducibility.
{
  description = "WriteOnce development environment (vscode + neovim + tmux + toolchain)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      devShells.${system}.default = pkgs.mkShell {
        name = "writeonce-dev";
        packages = with pkgs; [
          # Editors + multiplexer (the requested dev env). vscode is a heavy
          # Electron app — fine on the T450 but the slowest item to fetch/run.
          vscode
          neovim
          tmux
          # Tooling the configs + daily work expect.
          git
          lazygit
          lazydocker
          fzf
          ripgrep
          fd
          eza
          gnumake
          # Language servers for the vendored LazyVim config (mason also fetches
          # more at runtime; these cover the basics declaratively).
          lua-language-server
          nil               # Nix LSP
        ];
        shellHook = ''
          echo "WriteOnce dev env: nvim / vscode / tmux + git/lazygit/fzf/ripgrep/fd."
          echo "Editors read ~/.config (managed by Home Manager)."
        '';
      };
    };
}
