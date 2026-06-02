param([string]$Method = "Ping")
$pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'NidavellirCore', [System.IO.Pipes.PipeDirection]::InOut)
try {
    $pipe.Connect(4000)
    $writer = New-Object System.IO.StreamWriter($pipe)
    $writer.AutoFlush = $true
    $reader = New-Object System.IO.StreamReader($pipe)
    $writer.WriteLine('{"method":"' + $Method + '"}')
    $resp = $reader.ReadLine()
    Write-Output $resp
} catch {
    Write-Output "IPC-ERROR: $_"
} finally {
    $pipe.Dispose()
}
