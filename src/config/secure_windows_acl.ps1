$ErrorActionPreference = 'Stop'
$target = [Environment]::GetEnvironmentVariable('KAMISHIBAI_ACL_TARGET')
$kind = [Environment]::GetEnvironmentVariable('KAMISHIBAI_ACL_KIND')
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$administrators = [System.Security.Principal.SecurityIdentifier]::new([System.Security.Principal.WellKnownSidType]::BuiltinAdministratorsSid, $null)
if ([string]::IsNullOrWhiteSpace($target)) { exit 20 }
if ($kind -eq 'directory') {
    $item = [System.IO.DirectoryInfo]::new($target)
    $security = $item.GetAccessControl()
    $inheritance = [System.Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [System.Security.AccessControl.InheritanceFlags]::ObjectInherit
    $rule = [System.Security.AccessControl.FileSystemAccessRule]::new($sid, [System.Security.AccessControl.FileSystemRights]::FullControl, $inheritance, [System.Security.AccessControl.PropagationFlags]::None, [System.Security.AccessControl.AccessControlType]::Allow)
} elseif ($kind -eq 'file') {
    $item = [System.IO.FileInfo]::new($target)
    $security = $item.GetAccessControl()
    $rule = [System.Security.AccessControl.FileSystemAccessRule]::new($sid, [System.Security.AccessControl.FileSystemRights]::FullControl, [System.Security.AccessControl.AccessControlType]::Allow)
} else {
    exit 21
}
$owner = $security.GetOwner([System.Security.Principal.SecurityIdentifier])
if (($owner.Value -ne $sid.Value) -and ($owner.Value -ne $administrators.Value)) { exit 23 }
$security.SetAccessRuleProtection($true, $false)
$explicit = @($security.GetAccessRules($true, $false, [System.Security.Principal.SecurityIdentifier]))
foreach ($entry in $explicit) {
    [void]$security.RemoveAccessRuleSpecific($entry)
}
$security.AddAccessRule($rule)
$item.SetAccessControl($security)
$actual = $item.GetAccessControl()
if (-not $actual.AreAccessRulesProtected) { exit 22 }
$owner = $actual.GetOwner([System.Security.Principal.SecurityIdentifier])
if (($owner.Value -ne $sid.Value) -and ($owner.Value -ne $administrators.Value)) { exit 23 }
$rules = @($actual.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]))
if ($rules.Count -ne 1) { exit 24 }
$entry = $rules[0]
if ($entry.IdentityReference.Value -ne $sid.Value) { exit 25 }
if ($entry.AccessControlType -ne [System.Security.AccessControl.AccessControlType]::Allow) { exit 26 }
$full = [System.Security.AccessControl.FileSystemRights]::FullControl
if (($entry.FileSystemRights -band $full) -ne $full) { exit 27 }
