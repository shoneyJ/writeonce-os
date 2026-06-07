return {
  "mistweaverco/kulala.nvim",
  url = "git@github.com:shoneyJ/kulala.nvim.git",
  ft = { "http", "rest" },
  init = function()
    vim.filetype.add({ extension = { rest = "rest" } })
  end,
  keys = {
    { "<leader>R", "", desc = "+Rest", ft = { "http", "rest" } },
    { "<leader>Rb", "<cmd>lua require('kulala').scratchpad()<cr>", desc = "Open scratchpad", ft = { "http", "rest" } },
    { "<leader>Rc", "<cmd>lua require('kulala').copy()<cr>", desc = "Copy as cURL", ft = { "http", "rest" } },
    { "<leader>RC", "<cmd>lua require('kulala').from_curl()<cr>", desc = "Paste from curl", ft = { "http", "rest" } },
    {
      "<leader>Rg",
      "<cmd>lua require('kulala').download_graphql_schema()<cr>",
      desc = "Download GraphQL schema",
      ft = { "http", "rest" },
    },
    { "<leader>Ri", "<cmd>lua require('kulala').inspect()<cr>", desc = "Inspect current request", ft = { "http", "rest" } },
    { "<leader>Rn", "<cmd>lua require('kulala').jump_next()<cr>", desc = "Jump to next request", ft = { "http", "rest" } },
    { "<leader>Rp", "<cmd>lua require('kulala').jump_prev()<cr>", desc = "Jump to previous request", ft = { "http", "rest" } },
    { "<leader>Rq", "<cmd>lua require('kulala').close()<cr>", desc = "Close window", ft = { "http", "rest" } },
    { "<leader>Rr", "<cmd>lua require('kulala').replay()<cr>", desc = "Replay the last request", ft = { "http", "rest" } },
    { "<leader>Rs", "<cmd>lua require('kulala').run()<cr>", desc = "Send the request", ft = { "http", "rest" } },
    { "<leader>RS", "<cmd>lua require('kulala').show_stats()<cr>", desc = "Show stats", ft = { "http", "rest" } },
    { "<leader>Rt", "<cmd>lua require('kulala').toggle_view()<cr>", desc = "Toggle headers/body", ft = { "http", "rest" } },
    { "<leader><CR>", "<cmd>lua require('kulala').run()<cr>", mode = "n", desc = "Send the request", ft = { "http", "rest" } },
  },
  opts = {},
}
