$ErrorActionPreference = 'Stop'
$target = [Environment]::GetEnvironmentVariable('KAMISHIBAI_ACL_TEST_TARGET')
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$administrators = [System.Security.Principal.SecurityIdentifier]::new([System.Security.Principal.WellKnownSidType]::BuiltinAdministratorsSid, $null)
function Test-Private([System.Security.AccessControl.FileSystemSecurity]$acl) {
    if (-not $acl.AreAccessRulesProtected) { return $false }
    $owner = $acl.GetOwner([System.Security.Principal.SecurityIdentifier])
    if (($owner.Value -ne $sid.Value) -and ($owner.Value -ne $administrators.Value)) { return $false }
    $rules = @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]))
    if ($rules.Count -ne 1) { return $false }
    $rule = $rules[0]
    $full = [System.Security.AccessControl.FileSystemRights]::FullControl
    return $rule.IdentityReference.Value -eq $sid.Value -and $rule.AccessControlType -eq [System.Security.AccessControl.AccessControlType]::Allow -and ($rule.FileSystemRights -band $full) -eq $full
}
$file = [System.IO.FileInfo]::new($target).GetAccessControl()
$parent = [System.IO.Path]::GetDirectoryName($target)
$directory = [System.IO.DirectoryInfo]::new($parent).GetAccessControl()
if ((Test-Private $file) -and (Test-Private $directory)) { [Console]::Out.WriteLine('private') } else { exit 30 }
