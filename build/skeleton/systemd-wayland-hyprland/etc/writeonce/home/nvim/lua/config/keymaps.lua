-- Keymaps are automatically loaded on the VeryLazy event
-- Default keymaps that are always set: https://github.com/LazyVim/LazyVim/blob/main/lua/lazyvim/config/keymaps.lua
-- Add any additional keymaps here

-- <leader>e toggles FOCUS between explorer and editor (keeps explorer open).
-- Opens the explorer if it isn't open yet.
local function is_explorer_buf(buf)
  local ft = vim.bo[buf].filetype
  return ft == "snacks_picker_list" or ft == "snacks_picker_input"
end

local function toggle_explorer_focus()
  local explorer_win, editor_win
  for _, win in ipairs(vim.api.nvim_list_wins()) do
    local buf = vim.api.nvim_win_get_buf(win)
    if is_explorer_buf(buf) then
      explorer_win = win
    elseif vim.api.nvim_win_get_config(win).relative == "" then
      editor_win = editor_win or win
    end
  end

  if not explorer_win then
    Snacks.explorer()
    return
  end

  if is_explorer_buf(vim.api.nvim_get_current_buf()) then
    if editor_win then
      vim.api.nvim_set_current_win(editor_win)
    end
  else
    vim.api.nvim_set_current_win(explorer_win)
  end
end

vim.keymap.set({ "n", "t" }, "<leader>e", toggle_explorer_focus, { desc = "Toggle focus: explorer <-> editor" })
