param(
    [Parameter(Mandatory = $true)]
    [string[]]$args
)

cargo forgen
cargo @args
exit $LASTEXITCODE
