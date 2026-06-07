# /etc/writeonce/home/home.nix — the WriteOnce user environment (Home Manager).
# Ported faithful-light from ~/dotfiles; the dev toolchain (vscode/neovim/tmux/…)
# lives in the separate devShell at /etc/writeonce/devshell.
{ config, pkgs, lib, ... }:
{
  home.username = "writeonce";
  home.homeDirectory = "/home/writeonce";
  # Home Manager release this config targets; bump if `home-manager switch` warns.
  home.stateVersion = "25.05";

  # Daily desktop + shell packages (folds in the old desktop/flake.nix set).
  home.packages = with pkgs; [
    hyprland
    quickshell
    alacritty
    xdg-desktop-portal-hyprland
    wl-clipboard
    fish
    starship
    eza
  ];

  home.sessionVariables.EDITOR = "nvim";

  # ---- Terminal: alacritty (replaces kitty) ---------------------------------
  # Ported from ~/dotfiles/alacritty, minus the tmux-main.sh auto-launch (tmux
  # is in the devShell) and the Maximized startup (Hyprland tiles).
  programs.alacritty = {
    enable = true;
    settings = {
      env.TERM = "xterm-256color";
      window = {
        padding = { x = 8; y = 8; };
        opacity = 0.9;
        dynamic_padding = true;
        decorations = "Full";
      };
      font = {
        normal = { family = "JetBrainsMono Nerd Font"; style = "Regular"; };
        bold.style = "Bold";
        italic.style = "Italic";
        size = 12.5;
      };
      cursor = {
        style = { shape = "Beam"; blinking = "On"; };
        unfocused_hollow = true;
      };
      scrolling = { history = 10000; multiplier = 3; };
      colors = {
        primary = { background = "#1e1e2e"; foreground = "#cdd6f4"; };
        normal = {
          black = "#1e1e2e"; red = "#f38ba8"; green = "#a6e3a1"; yellow = "#f9e2af";
          blue = "#89b4fa"; magenta = "#f5c2e7"; cyan = "#94e2d5"; white = "#bac2de";
        };
        bright = {
          black = "#585b70"; red = "#f38ba8"; green = "#a6e3a1"; yellow = "#f9e2af";
          blue = "#89b4fa"; magenta = "#f5c2e7"; cyan = "#94e2d5"; white = "#a6adc8";
        };
      };
      keyboard.bindings = [
        { key = "N"; mods = "Control|Shift"; action = "CreateNewWindow"; }
      ];
    };
  };

  # ---- Shell + prompt -------------------------------------------------------
  # Ported from ~/dotfiles/fish + starship, without the workstation auto-tmux
  # exec or the ii-specific terminal-sequences cat. HM wires starship into fish.
  programs.fish = {
    enable = true;
    shellAliases = {
      ls = "eza --icons";
      q = "qs -c wo";          # thin WriteOnce shell (upstream dotfiles use `qs -c ii`)
      pamcan = "pacman";
    };
  };

  programs.starship = {
    enable = true;
    settings = {
      add_newline = true;
      character.success_symbol = "[➜](bold green)";
      package.disabled = true;
      directory = { truncation_length = 3; truncate_to_repo = true; style = "bold cyan"; };
      git_branch = { symbol = " "; style = "bold purple"; };
      git_status = {
        ahead = "⇡\${count}";
        behind = "⇣\${count}";
        diverged = "⇕⇡\${ahead_count}⇣\${behind_count}";
        conflicted = "=";
        untracked = "?\${count}";
        modified = "!\${count}";
        staged = "+\${count}";
        renamed = "»\${count}";
        deleted = "✘\${count}";
        stashed = "\$";
        style = "bold red";
      };
    };
  };

  # ---- tmux -----------------------------------------------------------------
  # Config managed here; the binary comes from the devShell. Ported from
  # ~/dotfiles/tmux/.tmux.conf; tpm plugins come from Nix, and the workstation-
  # absolute-path SSH/sesh binds are dropped.
  programs.tmux = {
    enable = true;
    prefix = "C-a";
    baseIndex = 1;
    keyMode = "vi";
    mouse = true;
    escapeTime = 0;
    historyLimit = 1000000;
    terminal = "tmux-256color";
    plugins = with pkgs.tmuxPlugins; [ sensible yank resurrect continuum cpu catppuccin ];
    extraConfig = ''
      set -g status-position top
      set -g renumber-windows on
      set -g detach-on-destroy off
      set -g focus-events on
      bind m break-pane
      bind G display-popup -E -w 90% -h 90% -d "#{pane_current_path}" "lazygit"
      bind D display-popup -E -w 90% -h 90% -d "#{pane_current_path}" "lazydocker"
      bind h select-pane -L
      bind j select-pane -D
      bind k select-pane -U
      bind l select-pane -R
      bind -n M-h previous-window
      bind -n M-l next-window
      bind -T copy-mode-vi v send -X begin-selection
      bind -T copy-mode-vi y send -X copy-pipe-no-clear
      set -g @continuum-restore 'on'
      set -g @resurrect-strategy-nvim 'session'
      set -g @catppuccin_flavor 'mocha'
    '';
  };

  # ---- Neovim (faithful) ----------------------------------------------------
  # The LazyVim tree vendored from ~/dotfiles/nvim. The nvim binary comes from
  # the devShell; lazy.nvim + mason fetch plugins + LSP servers at runtime
  # (impure — needs network on first launch; the store-symlinked config is
  # read-only, so update plugins by editing the source + re-switching).
  xdg.configFile."nvim" = { source = ./nvim; recursive = true; };

  # ---- Thin desktop configs (kept minimal; Home Manager symlinks them) ------
  xdg.configFile."hypr/hyprland.conf".source = ./hypr/hyprland.conf;
  xdg.configFile."quickshell/wo".source = ./quickshell/wo;

  programs.home-manager.enable = true;
}
