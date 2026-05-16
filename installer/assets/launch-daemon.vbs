' launch-daemon.vbs
' Avvia speedy-daemon.exe dalla stessa cartella, completamente nascosto (nessuna finestra console).
'
' Viene eseguito da wscript.exe tramite il collegamento nella cartella Startup:
'   wscript.exe /nologo "C:\...\Speedy\launch-daemon.vbs"
'
' WindowStyle 0 = SW_HIDE: il processo parte ma non apre nessuna finestra.
' bWaitOnReturn False: wscript.exe non aspetta che il daemon termini.

Option Explicit

Dim sh, dir, exe

Set sh = CreateObject("WScript.Shell")

' Ricava la cartella in cui si trova questo script
dir = Left(WScript.ScriptFullName, InStrRev(WScript.ScriptFullName, "\"))
exe = dir & "speedy-daemon.exe"

' Avvia il daemon nascosto e ritorna subito
sh.Run Chr(34) & exe & Chr(34), 0, False

Set sh = Nothing
