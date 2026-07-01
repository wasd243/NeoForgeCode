## Windows / pwsh port TODO

### Core (make it work well on Windows)
- [ ] Prefer pwsh 7+ over cmd.exe for execution (env.rs, with cmd fallback)
- [ ] Adjust command exec args for pwsh if needed (verify after shell swap)
- [ ] Make command generation produce pwsh-style commands on Windows (command_generator prompt)

### Integration (the pwsh `:` system — replaces zsh plugin)
- [ ] Port zsh shell-integration to a pwsh $PROFILE equivalent (forge_main/zsh/)
- [ ] Replace zsh-oriented user prompts/tips with pwsh ones

### Slim down (self-use build)
- [ ] Remove unused providers & telemetry (AWS/GCP/posthog/tracker)

### TUI

- [ ] Add TUI autocompletion
- [ ] Better rendering includes img, MD highlight, Vim keybind support, etc.

### Distribution
- [ ] npm wrapper package for `npm i -g` on Windows
