return {
  "olimorris/codecompanion.nvim",
  opts = {
    strategies = {
      -- Change the default chat adapter
      chat = {
        adapter = "ollama",
        inline = "ollama",
      },
    },
    adapters = {
      ollama = function()
        return require("codecompanion.adapters").extend("ollama", {
          name = "qwen", -- Give this adapter a different name to differentiate it from the default ollama adapter
          schema = {
            model = {
              default = "qwen2.5-coder:7b",
            },
          },
        })
      end,
    },

    log_level = "DEBUG",

    display = {
      diff = {
        enabled = true,
        close_chat_at = 240, -- Close an open chat buffer if the total columns of your display are less than...
        layout = "vertical", -- vertical|horizontal split for default provider
        opts = { "internal", "filler", "closeoff", "algorithm:patience", "followwrap", "linematch:120" },
        provider = "default", -- default|mini_diff
      },
    },
  },

  config = function(_, opts)
    require("codecompanion").setup(opts)
    vim.keymap.set(
      { "n", "v" },
      "<C-o>",
      "<cmd>CodeCompanionActions<cr>",
      { noremap = true, silent = true, desc = "CodeCompanion Actions" }
    )
    vim.keymap.set(
      { "n", "v" },
      "<leader>a",
      "<cmd>CodeCompanionChat Toggle<cr>",
      { noremap = true, silent = true, desc = "Toggle Chat Panel" }
    )
    vim.keymap.set(
      "v",
      "ga",
      "<cmd>CodeCompanionChat Add<cr>",
      { noremap = true, silent = true, desc = "Add Selection to Chat" }
    )

    -- Command-line abbreviation
    vim.cmd([[cabbrev cc CodeCompanion]])
  end,
}
