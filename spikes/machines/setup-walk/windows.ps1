<#
setup-walk/windows.ps1 - Phase 0 S5 on Windows: the setup checks of
docs/PLAN-MACHINES.md section 5.14 met in their worst states, then put back.

Run it yourself from an elevated PowerShell (Run as administrator), at the
repo root:

    powershell -NoProfile -ExecutionPolicy Bypass -File spikes\machines\setup-walk\windows.ps1

It asks before every change and restores every setting it changed, also when
it fails or you press Ctrl-C (a finally block):

  1. The network set Public, then back.
  2. The firewall: the current profile on (if it was off), block rules for
     mc-spike.exe as a cancelled "Allow access?" prompt leaves them, then the
     setup's fix (an allow rule for Private, the block rules removed), then
     all of it removed and the profile back as it was.
  3. Controlled Folder Access on, then back.
  4. A signed-out account, shown by `claude auth status` with an empty
     scratch CLAUDE_CONFIG_DIR - never by signing yours out.
  5. Long paths, Developer Mode, Git Bash: read only.

After each change `mc-spike checks` reads the state again. Results:
spikes\machines\out\setup-walk-windows.txt (git-ignored).
#>
$ErrorActionPreference = 'Stop'
$Machines = Split-Path -Parent $PSScriptRoot
$Out = Join-Path $Machines 'out'
$Bin = Join-Path $Machines 'mc-spike\target\release\mc-spike.exe'
$Log = Join-Path $Out 'setup-walk-windows.txt'
New-Item -ItemType Directory -Force $Out | Out-Null

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Error 'Run this from an elevated PowerShell (Run as administrator).'
}
if (-not (Test-Path $Bin)) {
    Write-Error 'Build the spike first: cargo build --release --manifest-path spikes\machines\mc-spike\Cargo.toml'
}

function Log([string]$Text) { $Text | Tee-Object -FilePath $Log -Append }
function Section([string]$Text) { Log ''; Log "== $Text" }
function Ask([string]$Question) {
    $a = Read-Host "$Question [y/N]"
    return ($a -eq 'y' -or $a -eq 'Y')
}
function Checks([string]$Name) {
    $file = Join-Path $Out "walk-$Name.json"
    & $Bin checks | Set-Content -Path $file -Encoding UTF8
    return (Get-Content $file -Raw | ConvertFrom-Json)
}
$RuleGroup = 'Mother Claude spike walk'

'' | Set-Content -Path $Log
Log "setup walk, $([System.Environment]::OSVersion.VersionString), $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mmZ'))"

# What the walk changed, so finally can put it back.
$restore = [ordered]@{}
try {
    # ---- 1. Network profile ------------------------------------------------
    Section '1. The network set Public'
    $net = Get-NetConnectionProfile | Select-Object -First 1
    Log "before: $($net.InterfaceAlias) is $($net.NetworkCategory)"
    if ($net.NetworkCategory -ne 'DomainAuthenticated' -and (Ask "Set $($net.InterfaceAlias) to Public for a moment?")) {
        $restore['profile'] = @{ Index = $net.InterfaceIndex; Category = [string]$net.NetworkCategory }
        Set-NetConnectionProfile -InterfaceIndex $net.InterfaceIndex -NetworkCategory Public
        $c = Checks 'public'
        Log "read while Public: $($c.networkProfiles.out)"
        Set-NetConnectionProfile -InterfaceIndex $net.InterfaceIndex -NetworkCategory $restore['profile'].Category
        $restore.Remove('profile')
        Log "back: $((Get-NetConnectionProfile -InterfaceIndex $net.InterfaceIndex).NetworkCategory)"
    } else { Log 'skipped' }

    # ---- 2. Firewall --------------------------------------------------------
    Section '2. The firewall, a cancelled prompt, then the fix'
    $category = [string](Get-NetConnectionProfile | Select-Object -First 1).NetworkCategory
    $fwProfile = if ($category -eq 'DomainAuthenticated') { 'Domain' } else { $category }
    $before = Get-NetFirewallProfile -Name $fwProfile
    Log "before: $fwProfile profile enabled=$($before.Enabled), default inbound=$($before.DefaultInboundAction)"
    if (Ask "Turn the $fwProfile firewall on (if off) and add block rules for mc-spike.exe for a moment?") {
        $restore['fwProfile'] = @{ Name = $fwProfile; Enabled = $before.Enabled }
        $restore['rules'] = $true
        Set-NetFirewallProfile -Name $fwProfile -Enabled True
        foreach ($proto in 'UDP', 'TCP') {
            New-NetFirewallRule -DisplayName "mc-spike (blocked, as a cancelled prompt leaves it) $proto" -Group $RuleGroup `
                -Program $Bin -Direction Inbound -Action Block -Protocol $proto -Profile Any | Out-Null
        }
        $c = Checks 'blocked'
        $mine = $c.firewallRulesForPrograms | Where-Object { $_.program -eq $Bin }
        Log "read while blocked: profiles $($c.firewallProfiles.out)"
        Log "rules for mc-spike.exe: $($mine.rules.out)"
        # The setup's fix: an allow rule on Private, Mother Claude's own block rules removed.
        Get-NetFirewallRule -Group $RuleGroup | Where-Object { $_.Action -eq 'Block' } | Remove-NetFirewallRule
        New-NetFirewallRule -DisplayName 'mc-spike (allowed by the setup fix) UDP' -Group $RuleGroup `
            -Program $Bin -Direction Inbound -Action Allow -Protocol UDP -Profile Private | Out-Null
        $c = Checks 'fixed'
        $mine = $c.firewallRulesForPrograms | Where-Object { $_.program -eq $Bin }
        Log "after the fix: $($mine.rules.out)"
    } else { Log 'skipped' }
    if ($restore.Contains('rules')) {
        Get-NetFirewallRule -Group $RuleGroup -ErrorAction SilentlyContinue | Remove-NetFirewallRule
        $restore.Remove('rules')
    }
    if ($restore.Contains('fwProfile')) {
        Set-NetFirewallProfile -Name $restore['fwProfile'].Name -Enabled $restore['fwProfile'].Enabled
        $restore.Remove('fwProfile')
        Log "firewall back: $fwProfile enabled=$((Get-NetFirewallProfile -Name $fwProfile).Enabled)"
    }

    # ---- 3. Controlled Folder Access -----------------------------------------
    Section '3. Controlled Folder Access'
    $cfa = (Get-MpPreference).EnableControlledFolderAccess
    Log "before: $cfa"
    if ($cfa -eq 0 -and (Ask 'Turn Controlled Folder Access on for a moment?')) {
        $restore['cfa'] = $cfa
        Set-MpPreference -EnableControlledFolderAccess Enabled
        $c = Checks 'cfa'
        Log "read while on: $($c.controlledFolderAccess.out) $($c.controlledFolderAccess.err)"
        Set-MpPreference -EnableControlledFolderAccess Disabled
        $restore.Remove('cfa')
        Log "back: $((Get-MpPreference).EnableControlledFolderAccess)"
    } else { Log 'skipped' }

    # ---- 4. A signed-out account --------------------------------------------
    Section '4. A signed-out account (an empty scratch config directory)'
    $scratch = Join-Path $env:TEMP ("mc-walk-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory $scratch | Out-Null
    $old = $env:CLAUDE_CONFIG_DIR
    try {
        $env:CLAUDE_CONFIG_DIR = $scratch
        $raw = & claude auth status --json 2>$null | Out-String
        $j = try { $raw | ConvertFrom-Json } catch { $null }
        Log "claude auth status with an empty directory: loggedIn=$($j.loggedIn)"
    } finally {
        $env:CLAUDE_CONFIG_DIR = $old
        Remove-Item -Recurse -Force $scratch -ErrorAction SilentlyContinue
    }

    # ---- 5. Read only ---------------------------------------------------------
    Section '5. Read only'
    $c = Checks 'final'
    Log "long paths: $($c.longPathsEnabled.out); Developer Mode: $($c.developerMode.out); Git Bash: $($c.gitBash | ConvertTo-Json -Compress)"
}
finally {
    if ($restore.Contains('profile')) {
        Set-NetConnectionProfile -InterfaceIndex $restore['profile'].Index -NetworkCategory $restore['profile'].Category
        Log 'restored the network profile'
    }
    if ($restore.Contains('rules')) {
        Get-NetFirewallRule -Group $RuleGroup -ErrorAction SilentlyContinue | Remove-NetFirewallRule
        Log 'removed the walk''s firewall rules'
    }
    if ($restore.Contains('fwProfile')) {
        Set-NetFirewallProfile -Name $restore['fwProfile'].Name -Enabled $restore['fwProfile'].Enabled
        Log 'restored the firewall profile'
    }
    if ($restore.Contains('cfa')) {
        Set-MpPreference -EnableControlledFolderAccess Disabled
        Log 'restored Controlled Folder Access'
    }
    Log ''
    Log "Results: $Log"
}
