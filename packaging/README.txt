Mentis {VERSION} – Diktieren auf Windows, direkt auf deinem PC
=============================================================

(English below.)

Taste halten, sprechen, loslassen: Der Text erscheint dort, wo gerade
der Cursor steht, egal in welchem Programm. Die Spracherkennung läuft
auf deinem PC. Nichts von dem, was du sagst, geht ins Internet.

Dies ist eine Vorabversion zum Ausprobieren.


STARTEN
-------

1. Die Zip-Datei entpacken: Rechtsklick > "Alle extrahieren ...".
   Ein beliebiger Ordner ist in Ordnung, z. B. D:\Programme\Mentis.
   Wichtig: Mentis nicht direkt aus der Zip-Datei starten, dann fehlen
   die mitgelieferten Dateien.

   Tipp: Vor dem Entpacken Rechtsklick auf die Zip-Datei > Eigenschaften
   > unten "Zulassen" anhaken > OK. Dann entfällt die Warnung aus Schritt 2.

2. mentis.exe doppelklicken.
   Windows warnt eventuell: "Der Computer wurde durch Windows geschützt".
   Das liegt daran, dass Mentis noch nicht signiert ist. Weiter mit
   "Weitere Informationen" > "Trotzdem ausführen".

   Startet mentis.exe gar nicht und es gibt kein "Trotzdem ausführen"?
   Dann ist unter Windows 11 die "Intelligente App-Steuerung" an
   (Windows-Sicherheit > App- und Browsersteuerung). Sie blockiert
   unsignierte Programme. Mentis läuft nur, wenn sie aus ist.

3. Beim ersten Start öffnet sich "Mentis einrichten". Mentis schlägt ein
   Sprachmodell passend zu deinem PC vor (mit Grafikkarte: Whisper,
   ohne: Parakeet). Unter "Speicherort" wählst du, wo es liegen soll.
   Es ist 260 bis 850 MB groß. Dann "Herunterladen".

4. Danach erscheint unten rechts kurz "Mentis ist bereit". Mentis läuft als
   kleines Symbol im Infobereich der Taskleiste. Windows 11 versteckt
   neue Symbole hinter dem Pfeil ^; von dort kannst du es in die
   Taskleiste ziehen.


UMSTIEG VON ECHO
----------------

Mentis hieß bis Version 0.7 "Echo". Wer Echo schon benutzt:

1. Echo beenden: Rechtsklick auf das Echo-Symbol > "Beenden".
2. Mentis wie oben entpacken und mentis.exe starten.

Beim ersten Start übernimmt Mentis Einstellungen, Sprachmodelle und
den Autostart von Echo; nichts muss neu geladen werden. Danach Echo
nicht mehr starten, sondern den alten Echo-Programmordner löschen.


DIKTIEREN
---------

- Rechte Strg-Taste halten, sprechen, loslassen.
- Für längere Texte: rechte Strg-Taste kurz antippen, sprechen, erneut
  antippen.
- Esc bricht eine Aufnahme ab.
- Über der Taskleiste zeigt ein kleines Feld, was Mentis gerade hört.
- In Fenster, die mit Administratorrechten laufen, kann Mentis nicht
  einfügen (Windows-Sperre).


EINSTELLUNGEN
-------------

Klick auf das Mentis-Symbol öffnet die Einstellungen: Taste, Mikrofon,
Schreibstil ("Sauber" ohne "äh" oder "Exakt" mit allem), Sprache,
Modell und mehr. Rechtsklick öffnet das Menü mit "Mit Windows starten",
"Protokoll öffnen", "Mentis entfernen ..." und "Beenden".


RÜCKMELDUNG
-----------

Wenn etwas nicht klappt: Rechtsklick auf das Symbol > "Protokoll öffnen"
und die Datei mitschicken. Das Protokoll enthält Zeiten, Modell und
Fehlermeldungen, aber nie den diktierten Text.


NEUE VERSIONEN
--------------

Mentis sieht einmal am Tag auf github.com nach, ob es eine neue Version
gibt. Wenn ja, erscheint eine Meldung und im Menü "Neue Version ...
herunterladen". Installiert wird nichts automatisch. Abschalten: in den
Einstellungen "Nach neuen Versionen suchen".


ENTFERNEN
---------

Rechtsklick auf das Symbol > "Mentis entfernen ...". Mentis löscht seine
Einstellungen, das Protokoll, den Autostart-Eintrag und die
heruntergeladenen Modelle. Andere Dateien bleiben. Danach den
Programmordner selbst löschen; Mentis öffnet ihn dafür im Explorer.

Was Mentis wo ablegt:
- Programm: der Ordner, in den du entpackt hast
- Einstellungen: %APPDATA%\Mentis
- Protokoll: %LOCALAPPDATA%\Mentis
- Sprachmodelle: der Speicherort aus "Mentis einrichten"


SPRACHMODELLE UND LIZENZEN
--------------------------

Die Modelle sind nicht in der Zip-Datei. Mentis lädt sie von
huggingface.co/handy-computer und prüft jede Datei mit einer Prüfsumme.
- Whisper large-v3-turbo und Whisper small: OpenAI, GGUF-Fassung von
  handy-computer, Apache-2.0
- Parakeet TDT 0.6B v3: NVIDIA, GGUF-Fassung von handy-computer,
  CC-BY-4.0 (https://creativecommons.org/licenses/by/4.0/)
Mentis selbst ist freie Software unter der MIT-Lizenz. Diese und die
Lizenzen der mitgelieferten Programmteile stehen in LICENSES.txt.


=============================================================
Mentis {VERSION} – dictation for Windows, right on your PC
=============================================================

Hold a key, speak, let go: the text appears wherever the cursor is, in
any program. Speech recognition runs on your PC; nothing you say is sent
to the internet. This is a preview version for trying it out.

GETTING STARTED
1. Unzip: right-click the zip file > "Extract All...". Any folder is
   fine. Do not start Mentis from inside the zip file.
   Tip: before unzipping, right-click the zip > Properties > tick
   "Unblock" > OK. This avoids the warning in step 2.
2. Double-click mentis.exe. If Windows says "Windows protected your PC"
   (Mentis is not signed yet), choose "More info" > "Run anyway".
   If nothing starts and there is no "Run anyway", Windows 11 "Smart App
   Control" is on (Windows Security > App & browser control). It blocks
   unsigned programs; Mentis runs only with it turned off.
3. On first start, "Set up Mentis" suggests a speech model for your PC
   (Whisper with a graphics card, Parakeet without). Choose where to save
   it under "Location" (260 to 850 MB), then click "Download".
4. "Mentis is ready" appears briefly. Mentis runs as an icon in the
   notification area; Windows 11 may hide it behind the ^ arrow.

COMING FROM ECHO
Mentis was called "Echo" up to version 0.7. Quit Echo first (right-click
its icon > "Quit"), then unzip and start Mentis as above. On first start
Mentis takes over Echo's settings, speech models and autostart, so nothing
is downloaded again. Afterwards do not start Echo again; delete its old
program folder.

DICTATING
- Hold the right Ctrl key, speak, release.
- Longer texts: tap right Ctrl, speak, tap again. Esc cancels.
- Mentis cannot type into windows running as administrator.

SETTINGS AND MENU
Click the Mentis icon for the settings (key, microphone, writing style,
language, model, ...). Right-click for the menu: start with Windows,
open log, remove Mentis, quit.

FEEDBACK
Right-click the icon > "Open log" and send the file. The log contains
timings, the model and error messages, never the dictated text.

NEW VERSIONS
Once a day Mentis asks github.com whether a new version exists. If so, a
notification and a menu entry "Download new version ..." appear. Nothing
is installed automatically. Turn it off in the settings ("Check for new
versions").

REMOVING MENTIS
Right-click the icon > "Remove Mentis ...". Mentis deletes its settings,
log, autostart entry and downloaded models; other files stay. Then
delete the program folder yourself; Mentis opens it in Explorer.
Data locations: settings %APPDATA%\Mentis, log %LOCALAPPDATA%\Mentis,
models in the location chosen during setup.

MODELS AND LICENSES
Models are downloaded from huggingface.co/handy-computer and verified
with a checksum: Whisper large-v3-turbo and small (OpenAI, GGUF by
handy-computer, Apache-2.0); Parakeet TDT 0.6B v3 (NVIDIA, GGUF by
handy-computer, CC-BY-4.0). Mentis itself is free software under the MIT
License. Its license and those of the included components are in
LICENSES.txt.
