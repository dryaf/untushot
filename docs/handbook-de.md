> German technical reference, preserved from the previous README. For the current English feature tour, language support and setup instructions, see [README](../README.md).

# UntuShot

Ein nativer Screenshot-Editor für **Linux/Wayland**, in Rust geschrieben und von
Flameshots Arbeitsablauf inspiriert. GTK 3 zeichnet die Oberfläche, Cairo rendert
Vorschau und Export mit demselben Code. Kein Browser und keine Electron-Laufzeit.

## Neuer Name: UntuShot

Seit 0.8.1 heißt das Projekt **UntuShot** (zuvor Rustshot). Der Terminalbefehl
lautet `untushot`, neue Projektdateien enden auf `.untushot`. Bestehende
`.rustshot`-Projekte bleiben lesbar. Neue Projekte verwenden Formatversion 4
mit editierbaren Rändern und Schatten für Bildebenen.

`make install` übernimmt bei lokaler Desktop-Installation eine erkannte frühere
Installation dieses Projekts:
Einstellungen, Autosicherung und Thumbnail-Cache wechseln in die entsprechenden
`untushot`-Verzeichnisse. Vorhandene UntuShot-Dateien werden dabei nicht
überschrieben. Der alte Hintergrundprozess wird vorher mit Sicherung beendet;
Tray, Autostart und `Strg+Shift+X` verwenden anschließend UntuShot. Die ursprünglichen
Tastenkürzel-Sicherungen bleiben für eine spätere Deinstallation erhalten.
Staging mit `DESTDIR` verändert keine laufende Sitzung. Eine fremde Installation
namens Rustshot wird nicht entfernt.

## Starten

Benötigt Rust **1.75+**, GTK **3.24+** samt Entwicklungsdateien und `pkg-config`.
Beispiel für Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libgtk-3-dev grim
# Für Tray und automatische GNOME-Anbindung:
sudo apt install libayatana-appindicator3-1 gnome-shell-extension-appindicator python3-gi
cargo build --release --locked
./target/release/untushot
```

`grim` ist optional und benötigt einen Compositor mit Screencopy-Unterstützung.
Ohne Unterstützung verwendet UntuShot das **XDG Screenshot Portal**. Dafür müssen
`xdg-desktop-portal` und das zur Desktopumgebung passende Backend installiert und
in der Benutzersitzung aktiv sein.

```sh
untushot --backend grim                 # Direkte Aufnahme, z. B. Sway
untushot --backend portal               # Desktop-Freigabedialog, z. B. GNOME/KDE
untushot --delay 3                      # Drei Sekunden im Aufnahme-Dialog vorbelegen
untushot --capture region               # Sofort aufnehmen, dann Bereich auswählen
untushot --capture region --delay 3     # Bereichsaufnahme mit drei Sekunden Vorlauf
untushot --capture full                 # Vollständige Aufnahme im Editor öffnen
untushot --capture system               # Systemauswahl über das Portal
untushot --list-monitors                # IDs, logische Positionen/Größen, GTK-Render-Skalierung
untushot --backend grim --monitor 2     # Zweiten Monitor als Quelle vorbelegen
untushot --open bild.png --windowed     # Vorhandenes Bild bearbeiten
untushot --output ~/Bilder/notiz.png    # Ziel für den Speichern-Dialog vorgeben
untushot --demo --windowed              # Zwei synthetische Displays zum Ausprobieren
```

Ohne vorhandene Sitzung startet UntuShot mit einer **Aufnahme-Seite im Fenster**.
Ansonsten wird die letzte freie Sitzung wiederhergestellt. `--demo` und `--open`
öffnen direkt den Editor; `F11` schaltet ins Vollbild. `--windowed` wird weiterhin
als kompatible Option akzeptiert. Ohne `--capture` beginnt die Aufnahme mit
„Aufnahme starten“. Schnellaktionen aus dem Tray und `Strg+Shift+X` überspringen
die Aufnahme-Seite; nach erfolgreicher Aufnahme folgt die Bereichsauswahl bzw. der Editor.

## Aufnahme → Bereich → Editor

1. **Aufnahmeart:** „Bereich“, „Ganzer Bildschirm“ oder „Systemauswahl“.
2. **Quelle:** Gesamter Desktop oder ein Monitor bei direkter Aufnahme. Auf
   GNOME/KDE führt „Systemauswahl“ in den Desktopdialog, dessen Auswahlmöglichkeiten
   vom installierten Portal-Backend abhängen.
3. **Timer:** 0 bis 60 Sekunden; 0 startet sofort. Ein Countdown lässt sich abbrechen.
4. **Aufnehmen:** UntuShot blendet sich aus. Bei „Bereich“ wird zunächst nur der
   eingefrorene Screenshot in einer eigenen, abgedunkelten Auswahlansicht gezeigt.
   Werkzeug- und Exportleisten sind ausgeblendet. Ausschnitt aufziehen und mit
   „Im Editor öffnen“ / Enter bestätigen. Acht Griffe ändern die Größe, Ziehen im
   Inneren verschiebt den Ausschnitt. Die zuschaltbare Pixellupe zeigt das Original
   zehnfach vergrößert. Esc bricht zuerst eine laufende Geste ab, sonst geht es zurück.
5. **Bearbeiten:** Werkzeuge links, Bild in der Mitte, Ebenen und Eigenschaften
   rechts. Der gewählte Ausschnitt wird passend eingepasst. „Zuschneiden“ zeigt
   wieder die vollständige Aufnahme. Die breite Exportbereich-Auswahl hat ihre eigene Zeile.
6. **Ausgeben:** PNG/Kopieren für das fertige Bild oder Projekt für spätere Änderungen.

Der Timer ist eine Startverzögerung, kein Serienaufnahme-Intervall. Eine neue
Aufnahme ersetzt das Dokument erst nach Erfolg. Bei nicht gespeicherten Änderungen
fragt UntuShot vor dem Ersetzen nach. Beim Schließen wird die Sitzung automatisch
gesichert; wenn das nicht gelingt, bleibt das Fenster mit einer Fehlermeldung offen.
Ohne verfügbare Autosicherung bleibt die Rückfrage beim Schließen erhalten.

## Multimonitor

**Mit grim:** Alle angeschlossenen Monitore werden in einem gemeinsamen Bild
aufgenommen. Monitorpositionen und logische Größen kommen von GDK/Wayland. Das
Bounding Rectangle berücksichtigt Displays links/oberhalb des Ursprungs,
Hochkant-Displays, versetzte Anordnungen und Zwischenräume. Die Aufnahme verwendet
die größte von GDK gemeldete Skalierung: alle Monitore liegen dadurch auf einem
einheitlichen Pixelraster. Niedriger skalierte Monitore werden dabei hochskaliert.
GDK 3 meldet ganzzahlige Skalierungsfaktoren; bei Fractional Scaling kann das Raster
größer als die physische Auflösung sein. Die Ausgabe ist kein natives 1:1-Abbild
jedes einzelnen Displays bei gemischten Skalierungen.

Die anklickbare Monitorübersicht zeigt die tatsächliche relative Anordnung inklusive
Hochkant-Displays und negativer Positionen. Bei direkter Aufnahme wählt ein Klick die
Quelle; mit dem Portal wird die Systemauswahl aktiviert, in der du den Monitor
noch bestätigst. Die Quellenauswahl wird beim Öffnen der Aufnahme-Seite aktualisiert.

Bei bekannter Monitorzuordnung öffnen im Editor das Monitor-Icon und das breite
Monitor-Menü die Exportbereich-Wahl. Ohne Zuordnung wird das leere Monitor-Popup ausgeblendet.
Ein Klick auf einen Monitor in der Übersicht wählt dessen gesamten Bildbereich. Alternativ
einen beliebigen Ausschnitt **über Monitorgrenzen hinweg** aufziehen. Gestrichelte
Monitorgrenzen sind Orientierungshilfen und erscheinen nicht im Export.
Monitor-IDs entsprechen der aktuellen GDK-Reihenfolge; bei Umstecken neu prüfen.

Die Aufnahmeübersicht aktualisiert ihre Anordnung ungefähr einmal pro Sekunde.
Eine Quelle bleibt nur bei eindeutiger Übereinstimmung von Name, Position, Größe
und Skalierung ausgewählt. Eine neue Reihenfolge allein verwechselt keine Monitore;
bei fehlender oder mehrdeutiger Zuordnung wird auf die gesamte Quelle zurückgestellt.

Ändert sich die Anordnung während Countdown oder Aufnahme, wird die Aufnahme
gestoppt und die aktualisierte Quellenauswahl angezeigt. Das bisherige Dokument
bleibt erhalten. Zusätzlich prüft das Capture-Backend die vor dem Timer festgehaltene
Anordnung vor der Aufnahme und die Anordnung nach der Aufnahme. Auch ein Abbruch
vor dem ersten Timer-Tick darf das wieder sichtbare Fenster nicht erneut verstecken.

Eine nicht direkt verfügbare `--monitor`-Vorgabe startet keine Aufnahme des gesamten
Desktops als Ersatz, sondern zeigt einen Hinweis zur Quellenauswahl. Bei
**Systemauswahl** bestimmt der Desktopdialog den Bildumfang; eine frühere direkte
Monitorwahl beschneidet sein Ergebnis nicht nachträglich. Wird eine direkte Aufnahme
auf das Portal zurückgeführt und fehlt die Zuordnung, erklärt die Bereichsauswahl,
warum der gewünschte Ausschnitt manuell gewählt werden muss.

Für 0.7.6 wurde die lokale Anordnung ohne Bildschirmaufnahme aus GNOME und GTK
ausgelesen: zwei 4K-Displays bei 150 %, rechts Hochformat, links vertikal versetzt.
Die logischen Rechtecke sind `(2560, 0, 1440, 2560)` und `(0, 752, 2560, 1440)`.
GTK meldet Render-Skalierung 2; das bedeutet hier **nicht** Desktop-Skalierung 200 %.
Diese Geometrie ist als Regressionstest für bekannte Aufnahme-Raster abgedeckt.
GTK-Tests simulieren zusätzlich Umordnung, Abbruch mit/ohne Timer, fehlende
Portal-Zuordnung und die Systemauswahl. Eine echte Aufnahme- und Hotplug-Abnahme
sowie unterschiedliche Skalierungen zwischen den Displays bleiben offen.

Bei jeder Aufnahme wird das Layout neu gelesen.

**Mit dem Portal:** Der Desktop bestimmt, ob ein gesamter Desktop, ein Display
oder ein gewählter Ausschnitt geliefert wird. Das Screenshot-Portal liefert keine
Monitor-Koordinaten. UntuShot bietet deshalb bei Portal-Aufnahmen keine vermeintlich
exakten Monitor-Presets an. Mehrere im Bild enthaltene Displays können gemeinsam
bearbeitet und beschnitten werden. Wenn bei einer direkten Monitor-Auswahl auf das
Portal zurückgefallen wird, öffnet UntuShot die manuelle Bereichsauswahl.

Der Editor stellt das gemeinsame Bild **in einem Fenster** dar und lässt sich auf
einen beliebigen Monitor verschieben. Er ist kein Satz synchroner Auswahl-Overlays
über allen physischen Displays. „Einpassen“ zeigt die gesamte Anordnung;
Vergrößern und Scrollen erlauben präzises Zeichnen auf großen Desktops.
Die Auswahl verwendet eine zwischengespeicherte Vorschau in der Auflösung des
Fensters. Beim Ziehen wird das große Original nicht jedes Mal neu skaliert;
Original, Pixellupe und Export behalten die volle Auflösung. Änderungen, Zoom,
Fenstergröße und Displayskalierung erneuern die Vorschau.

## Werkzeuge und Bedienung

| Taste | Aktion |
| --- | --- |
| `V` | Objekt auswählen, verschieben oder an acht Griffen skalieren |
| `1` | Zuschneiden: acht Griffe, innen verschieben, Shift+Ziehen für einen neuen Bereich |
| `Pfeiltasten` / `Shift+Pfeiltasten` | Ausschnitt oder ausgewählte Objekte um 1 / 10 Pixel verschieben |
| `Strg+Pfeiltasten` | Beim Zuschneiden rechte/untere Kante um 1 Pixel ändern; mit Shift um 10 |
| `Strg+I` | PNG/JPEG als Bildebene rechts anfügen |
| `Strg+D` | Ausgewählte Objekte duplizieren |
| `F2` | Einzelne Ebene umbenennen; Enter übernimmt |
| `F8` / `F9` / `Tab` | Werkzeuge / Ebenen / beide Seitenleisten ein- und ausklappen |
| `2` | Freihandstift |
| `3` / `4` | Linie / Pfeil |
| `5` / `6` | Rechteck / Ellipse |
| `7` | Transparenter Textmarker |
| `8` | Deckend schwarz schwärzen |
| `9` | Text direkt im Bild anlegen; Enter übernimmt, Shift+Enter erzeugt eine neue Zeile |
| `Strg+Z` | Rückgängig, einschließlich Bereichsauswahl |
| `Strg+Umschalt+Z` / `Strg+Y` | Wiederholen |
| `Strg+A` | Im Auswahlwerkzeug den sichtbaren Ausschnitt markieren; sonst den Exportbereich auf das ganze Bild setzen |
| `Strg+S` | Bestätigten PNG-Export aktualisieren; beim ersten Speichern Ziel wählen |
| `Strg+Umschalt+S` | PNG speichern unter …, mit Überschreibbestätigung |
| `Strg+Alt+S` | Geöffnetes/gespeichertes Projekt aktualisieren; sonst Ziel wählen |
| `Strg+Alt+Umschalt+S` | Projekt speichern unter …, mit Überschreibbestätigung |
| `Strg+O` | Bild oder Projekt öffnen |
| `S` | Einrasten beim Verschieben ein-/ausschalten |
| `Alt` beim Verschieben | Vorübergehend ohne Einrasten bewegen |
| `M` | Rechteckige Pixelauswahl aufziehen; Exportbereich bleibt unverändert |
| `Strg+J` | Sichtbare Pixelauswahl als neue Bildebene kopieren |
| `Strg+V` | Im Editor ein Bild als neue Ebene einfügen; in Textfeldern normal Text einfügen |
| `Strg+Umschalt+V` | Bildimport mit Vorschau auch bei Fokus in einem Textfeld |
| `Strg+W` | Fenster schließen; mit aktivem Tray im Hintergrund weiterlaufen |
| `Strg+Q` | Sitzung sichern und UntuShot einschließlich Tray beenden |
| `Entf` | Ausgewählte Ebenen löschen |
| `Strg+C` | Im Auswahlwerkzeug die Pixelauswahl kopieren; sonst den Exportausschnitt |
| `Strg+N` | Neue Aufnahme |
| `Strg+Mausrad` | Am Mauszeiger zoomen |
| `Leertaste+Ziehen` / mittlere Maustaste | Ansicht verschieben, ohne Objekte zu verändern |
| `Mausrad` / `Shift+Mausrad` | Ansicht vertikal / horizontal verschieben |
| `+` / `−` / `0` | Vergrößern / verkleinern / einpassen und Verschiebung zurücksetzen |
| `Enter` | Laufende Aktion bestätigen; ausgewählten Text bearbeiten; Aufnahme starten |
| `F11` | Vollbild umschalten |
| `Esc` | Aktuelle Textänderung/Geste abbrechen, dann Zuschneiden verlassen oder Objektauswahl lösen; schließt das Hauptfenster nicht |

Farbe und Strichstärke werden links eingestellt. Text besitzt eine eigene
Schriftgröße in Bildpixeln (6–512 px), unabhängig von der Strichstärke. Im Textmodus
ersetzt dieses Feld die Linienstärke. Der zusätzliche Pfeilbutton neben Plus/Minus
öffnet eine scrollbare Auswahl mit „Sample“ in 16 gängigen Größen (6–96 px),
als Vorschau bei 75 % und unabhängig vom Canvas-Zoom. Die aktuelle Größe steht
im Menütitel; beim Öffnen wird die passende Vorschau automatisch sichtbar.
Ein Klick übernimmt die Größe; beliebige Werte bis 512 px bleiben direkt eingebbar.
Enter übernimmt auch einen gerade eingegebenen Zahlenwert; Esc verlässt das
Eingabefeld. Ungültige oder außerhalb des erlaubten Bereichs liegende Eingaben
bei Schriftgröße und Linienstärke behalten den letzten gültigen Wert bei.
Beim Ebenen- oder Werkzeugwechsel und vor dem Duplizieren werden offene Werte
noch für die bisher ausgewählte Ebene übernommen. Strg+Z funktioniert auch
direkt im Schriftgrößenfeld und im Feld für die eigene Linienstärke; eine noch
eingetippte Änderung wird dabei zuerst übernommen und dann zurückgenommen,
sodass Wiederholen sie wiederherstellen kann.
Die Statuszeile oben zeigt die tatsächlichen Exportmaße in Pixeln.
Bis zu 100 Bearbeitungsschritte sind rückgängig machbar. Die Aufnahme bleibt dabei
unverändert; erst der Export rendert die Annotationen und beschneidet das Bild.
Schwärzen schreibt vollständig deckende schwarze Pixel in den PNG-Export.

Die Zwischenablage verwendet GTK und funktioniert auch ohne `wl-copy`.
**Bis zum Einfügen weiterlaufen lassen**, sofern kein Clipboard-Manager die Bilddaten
übernimmt. Das Ausblenden ins Tray ist möglich; „UntuShot beenden“ gibt die
Zwischenablage frei. Nach dem Kopieren beendet sich UntuShot nicht automatisch.

## Bilddetails als eigene Ebene hervorheben

1. **Auswahl (M)** aktivieren und ein Rechteck aufziehen. Griffe und Pfeiltasten
   passen die Auswahl an; Shift+Ziehen beginnt eine neue. Der Exportausschnitt
   bleibt unverändert. Esc bricht erst die Geste ab, danach hebt es die Auswahl auf.
2. **Als neue Ebene** oder **Strg+J** kopiert die sichtbaren Pixel einschließlich
   sichtbarer Annotationen. Die Originale bleiben erhalten. Alternativ mit
   **Strg+C** kopieren und mit **Strg+V** als neue Bildebene einfügen.
3. Die neue Ebene verschieben und an den acht Griffen proportional vergrößern.
   Rechts bei **Rand** und **Schatten** Randfarbe und -stärke sowie Schatten mit
   Weichheit, Versatz und Deckkraft einstellen. Alle gültigen Zahlenwerte werden
   direkt beim Tippen oder mit den Pfeiltasten im Bild sichtbar. Die Schattenwerte
   stehen bei aktiviertem Schatten direkt unter dem Schalter.
4. Als Projekt speichern, um Bilddaten, Ebenen und Effekte bearbeitbar zu behalten.
   PNG-Export und Zwischenablage enthalten das gerenderte Ergebnis.

**Als neue Ebene einfügen** im Dateimenü übernimmt auch Bilder aus anderen Apps.
Zu große Bilder werden in den aktuellen Ausschnitt eingepasst; ihre ursprünglichen
Bilddaten bleiben erhalten. Kopieren auf eine neue Ebene, Einfügen und Ändern der
Effekte unterstützen Undo/Redo. Die Pixelauswahl selbst ist vorübergehend und wird
nicht im Projekt gespeichert.

Eine zusammenhängende Bearbeitung desselben Zahlenfelds ist ein Undo-Schritt.
Enter oder das Verlassen des Felds schließt diesen Schritt ab; Strg+Z funktioniert
auch im fokussierten Eigenschaftenfeld. Speichern und andere Dokumentaktionen
trennen Bearbeitungsschritte. Zurücksetzen auf den Ausgangswert erzeugt keinen
zusätzlichen Undo-Schritt. Ungültige oder außerhalb des erlaubten Bereichs liegende
Eingaben verändern das Bild nicht.

## Präzise platzieren und Objektaktionen

**Einrasten · S** links unten richtet verschobene Objekte an Kanten und Mitten
anderer sichtbarer Ebenen sowie am aktuellen Bildausschnitt aus. Die Einstellung
wird gespeichert. Der Fangabstand bleibt beim Zoomen gleich groß auf dem Bildschirm.
**Alt** setzt das Einrasten während des Ziehens vorübergehend aus. Bei mehreren
ausgewählten Ebenen wird die gemeinsame Begrenzung ausgerichtet; die Abstände
innerhalb der Gruppe bleiben erhalten. Ausgeblendete und vollständig außerhalb
des Ausschnitts liegende Ebenen ziehen die Auswahl nicht an.

Hilfslinien zeigen die eingerastete Ausrichtung nur während des Verschiebens.
Sie werden weder exportiert noch im Projekt gespeichert. Esc verwirft die laufende
Bewegung; Strg+Z macht den abgeschlossenen Zug als einen Schritt rückgängig.
Zahlenfelder, Pfeiltasten und Skalierungsgriffe bleiben für freie, genaue Änderungen
verfügbar; das Einrasten gilt für das Verschieben mit der Maus.

**Rechtsklick auf ein Objekt** öffnet Duplizieren, eine Ebene nach vorne/hinten und
Löschen; Texte bieten zusätzlich **Text bearbeiten**. Rechtsklick auf ein bereits
ausgewähltes Gruppenmitglied erhält die Mehrfachauswahl für Duplizieren und Löschen.
Die Reihenfolge lässt sich im Menü bei einer einzelnen Ebene ändern. Alle Aktionen
unterstützen Undo/Redo. Rechtsklick auf die freie Fläche verändert keine Ebene.

## Strichstärke mit Vorschau

Der Button neben **Stärke** zeigt den aktuellen Pixelwert. Sein Popup bietet
zwölf Linienmuster von 1 bis 100 px; ein Klick übernimmt den Wert und schließt
die Auswahl. Alle Muster haben denselben Maßstab **1:2**, unabhängig vom Canvas-Zoom.
Unter „Eigene Stärke“ sind exakte Werte von 1 bis 100 px mit Live-Vorschau möglich.
Die gewählte Stärke gilt für neue Striche bzw. ausgewählte Objekte; Änderungen
an Objekten bleiben rückgängig machbar. Bloßes Öffnen oder Überfahren der Muster
ändert nichts. Text verwendet weiterhin seine separate Schriftgröße.

Die **Farbauswahl** öffnet sich ebenfalls direkt am Button: Objektfarbe neben
den Zeichenwerkzeugen, Randfarbe neben den Ebenenoptionen. Die Palette und der
Editor für eigene Farben bleiben im selben Popup. „Übernehmen“ bestätigt die
Farbe; Abbrechen, Escape oder ein Klick außerhalb verwerfen die Auswahl.

## Sicherheitsprüfung

`make audit` prüft die lokalen GTK3-Rückports, führt die optimierten
Sicherheits-Regressionstests aus und scannt Cargo.lock mit `osv-scanner`.
Der dokumentierte GLib-Rückport erfordert eine zeitlich begrenzte OSV-Ausnahme,
weil der Scanner nur die alte Versionsnummer kennt. Herkunft, Änderungen,
Verifikation und die verbleibende GTK3-Wartungsgrenze stehen in
[**vendor/README.md**](../vendor/README.md). Ungepflegte `adler`- und
`proc-macro-error`-Abhängigkeiten wurden entfernt; `anyhow` wurde korrigiert.

## Dateimenü, „Öffnen mit“ und Screenshot-Verlauf

Strg+O öffnet die Dateiauswahl auch aus einem Text-, Ebenennamen- oder
Größenfeld heraus. Offene Eingaben werden übernommen; beim Abbrechen der
Dateiauswahl bleiben das Dokument und sein Exportziel erhalten.

Im Editor stehen **PNG exportieren** und **Kopieren** direkt oben rechts.
Der Export verwendet dasselbe bestätigte Ziel wie Strg+S; beim ersten Export
erscheint der Dateidialog. Unten steht der **Projektstatus** (ungespeichert,
geändert oder gespeichert) direkt neben **Projekt speichern**. Ein Klick auf den
Status zeigt zusätzlich PNG-Ziel und Autosicherung; Sicherungsfehler bleiben
auch ohne Öffnen dieser Details sichtbar.

Das kleine Menü links oben bündelt **Bild oder Projekt öffnen …**, **Bild als neues Dokument …**, **PNG speichern**, **PNG speichern
unter …**, **Projekt öffnen …**, **Projekt speichern** und **Projekt speichern unter …**.
**Projekt öffnen …** steht direkt bei den Projekt-Speicheraktionen und zeigt zunächst
Projektdateien (`.untushot` und ältere `.rustshot`) an. Dabei werden alle gespeicherten
Ebenen wieder einzeln bearbeitbar geladen. **Strg+O** öffnet weiterhin Bilder und Projekte.
Beim ersten PNG-Export erscheint der
Dateidialog; danach aktualisiert „Speichern“ genau dieses bestätigte Ziel.
„Speichern unter …“ öffnet den Dialog jedes Mal. Speichern, Fenster schließen und
Beenden funktionieren auch bei Fokus in Text- und Zahlenfeldern; Strg+S übernimmt
einen gerade eingegebenen Annotationstext vor dem Export. Das Öffnen eines Bildes setzt
kein automatisches Überschreibziel: Ein Original wird erst überschrieben, wenn
du es ausdrücklich im Speicherdialog als Ziel bestätigst.

Ein geöffnetes oder bereits gespeichertes Projekt wird mit **Strg+Alt+S** direkt
aktualisiert. **Projekt speichern unter …** wählt eine neue Datei; Abbrechen oder
Schreibfehler behalten das bisherige Ziel und die Bearbeitung. Beim Bildwechsel
wird das Projektziel zurückgesetzt. Vorgeschlagene neue Dateinamen enthalten ein
lesbares lokales Datum und die Uhrzeit, z. B. `UntuShot-2026-10-30_14-05-09.png`.

Nach `make install` wird UntuShot unter Ubuntus **„Öffnen mit“ für PNG und JPEG**
angeboten. Der bisherige Standardbetrachter wird nicht verändert. Auch `untushot
"/pfad/Bild mit Leerzeichen.png"` funktioniert; `--open` bleibt verfügbar.

Standardziel ist **Screenshots im XDG-Bilderordner**, z. B. `~/Pictures/Screenshots`
oder `~/Bilder/Screenshots`. Der Ordner wird bei Bedarf beim Speichern oder über
„Ordner öffnen“ angelegt. Ein ausdrücklich gewählter anderer Exportordner wird
weiterhin gemerkt; `--output` hat Vorrang.

Unter der Bildfläche zeigt eine einklappbare Leiste die **60 zuletzt geänderten
PNG-/JPEG-Dateien** aus diesem Screenshot-Ordner. Werkzeuge und Ebenen behalten
ihre volle Höhe, auch bei aufgeklapptem Verlauf. Auf der Aufnahme-Seite steht
die Leiste weiterhin unten über die Fensterbreite zur Verfügung; Scrollposition
und eingeklappter Zustand bleiben beim Wechsel erhalten.
Ein Klick öffnet das Original zur Bearbeitung; bei
ungespeicherten Änderungen bietet die Rückfrage Bild-Speichern, Projekt-Speichern,
Nicht speichern oder Zurück. Ist das PNG bereits aktuell, bestätigt die Rückfrage
„Bild gespeichert“ und fragt nur noch nach einem Projekt für die spätere Bearbeitung.
„Ohne Projekt weiter“ schließt diese Bearbeitung; die gespeicherte PNG-Datei bleibt erhalten. Strg+S speichert auch direkt aus dieser Rückfrage; Abbrechen
oder ein Schreibfehler verhindert den Bildwechsel. Ein PNG bewahrt das fertige
Bild, ein Projekt zusätzlich die editierbaren Ebenen. Die Leiste aktualisiert
sich nach Exporten und bei geöffnetem Fenster und aufgeklapptem oder leerem Verlauf ungefähr
alle fünf Sekunden. Solange keine Bilder vorhanden sind, ist die gesamte Leiste
ausgeblendet; das erste Bild lässt sie automatisch erscheinen.
Während der Bereichsauswahl ist sie ausgeblendet. Ihre eingeklappte Einstellung
wird gespeichert. Projekte und ungespeicherte Aufnahmen sind nicht Teil dieser
Dateiliste; die separate Sitzungswiederherstellung bleibt erhalten.

Neue oder geänderte Dateien aktualisieren nur die betroffenen Vorschauen.
Die übrigen Kacheln bleiben erhalten. Beim Durchsehen älterer Bilder bleibt die
sichtbare Stelle auch beim Hinzufügen oder Entfernen von Aufnahmen erhalten;
am Anfang der Leiste erscheinen weiterhin die neuesten Bilder.

Vorschaubilder entstehen auf einem Hintergrundthread. Der private Cache liegt
unter `$XDG_CACHE_HOME/untushot/thumbnails-v1`, standardmäßig
`~/.cache/untushot/thumbnails-v1`, mit höchstens **256 Vorschaubildern**. Dateipfad,
Änderungszeit und Größe bestimmen die Wiederverwendung; beschädigte Cachedateien
werden neu erzeugt. Große oder unlesbare Bilder erhalten bei fehlender Vorschau
ein Ersatzsymbol. Den Cache kann man bei beendetem UntuShot löschen; die Originale
bleiben erhalten. `--demo` liest weder persönliche Screenshots noch deren Cache.

## Editierbare Ebenen und Projekte

Jeder Strich, Pfeil, jede Form und jeder Text ist eine eigene Ebene. Das Original
bleibt unverändert. Objekte lassen sich im Bild mit `V` oder über die Ebenenliste
anwählen. Die oberste Listenzeile entspricht dem vordersten Objekt.
Die Trennlinie links neben der Ebenenleiste lässt sich ziehen, um lange Namen
besser lesen zu können. Die Breite bleibt beim Ein-/Ausblenden erhalten und wird
mit den Einstellungen gespeichert. Unten behalten die Ebenenoptionen ihre
benötigte Höhe; die Liste darüber nutzt den übrigen Platz und scrollt unabhängig.

- Ein normaler Klick in der Ebenenliste wählt genau eine Ebene. Strg-Klick fügt
  Ebenen zur Auswahl hinzu oder wählt sie wieder ab; Shift-Klick wählt einen
  zusammenhängenden Bereich. Esc hebt die Auswahl auf.
- Beim Verkleinern des Fensters bleibt eine zuvor sichtbare, einzeln ausgewählte
  Ebene im sichtbaren Listenbereich. Manuelles Scrollen weg von der Auswahl
  bleibt erhalten.
- Shift-/Strg-Klick im Bild wählt mehrere Objekte aus.
- Gemeinsam verschieben, duplizieren, löschen sowie Farbe/Stärke und Sichtbarkeit ändern.
- Mehrfachauswahl links, rechts, oben, unten oder mittig an ihrer gemeinsamen
  Begrenzung ausrichten; die Ausrichtungsaktionen erscheinen bei mehreren Objekten.
- Einzelne Objekte an acht Eck- und Seitengriffen skalieren und mit F2 umbenennen.
  Der Mauszeiger zeigt die jeweilige Richtung; beim Verschieben erscheint ein
  Bewegungskreuz. Die Griffe bleiben beim Zoomen gleich groß und sind auch am
  Bildrand vollständig bedienbar. Die gegenüberliegende Ecke bzw. Seitenmitte
  bleibt beim Skalieren fest. Bilder und Text behalten ihre Proportionen;
  Formen lassen sich in Breite und Höhe unabhängig verändern. Esc verwirft die
  laufende Ziehbewegung, Strg+Z nimmt die gesamte abgeschlossene Bewegung zurück.
- Bei einer einzelnen ausgewählten Ebene **Position und Größe** im
  Eigenschaftenbereich aufklappen, exakte X/Y-Position und Breite/Höhe einstellen
  — gültige Zahlenwerte wirken sofort. Bilder und Text bleiben beim Ändern
  von Breite oder Höhe proportional. **Text ändern** erscheint nur für Textebenen.
- Farbe und Strich-/Textgröße links ändern; Sichtbarkeit rechts umschalten.
- Jede Ebenenzeile zeigt eine kleine Inhaltsvorschau für Bilder, Text und Formen.
  Transparente Bereiche erscheinen auf einem Schachbrett. Ausgeblendete Ebenen
  behalten ihre Vorschau und ihren Namen in abgeschwächter Darstellung.
- Das **Auge direkt in der Ebenenzeile** blendet genau diese Ebene ein oder aus,
  ohne sie vorher auszuwählen oder eine bestehende Mehrfachauswahl zu ändern.
  Jeder Klick ist ein eigener Undo-Schritt. Die Sichtbarkeit wird im Projekt gespeichert.
- Einzelne Ebenen an der Zeile oder am Griff **⠿** ziehen, um die Reihenfolge
  zu ändern. Beim Halten der Maustaste (nach 150 ms) oder beim Beginn des Ziehens zeigen
  Linien die möglichen Einfügepositionen;
  beim Ziehen wird das aktuelle Ziel hervorgehoben. Am oberen und unteren Listenrand
  scrollt die Liste automatisch und wird zum Rand hin schneller. Die Einfügelinie
  bleibt bei kleinen Mausbewegungen stabil; die Zeilen behalten ihre Größe.
  Die verschobene Ebene bleibt ausgewählt, die
  gesamte Verschiebung ist ein Undo-Schritt. Abbrechen oder Ablegen außerhalb
  verändert nichts. Alternativ die Pfeilbuttons verwenden; der Papierkorb löscht.
- Doppelklick oder F2 bearbeitet den Namen direkt in der Ebenenzeile und markiert
  den bisherigen Namen. Enter oder ein Fokuswechsel übernimmt den Namen;
  Escape verwirft die Eingabe. Umbenennen ist ein einzelner Undo-Schritt.
  Strg+Z oder der Rückgängig-Button nehmen auch einen noch nicht bestätigten
  Namen zurück; Wiederholen stellt ihn wieder her. Die Ebene bleibt erhalten.
  Während der Namenseingabe ist Wiederholen inaktiv, und ein unveränderter
  Name nimmt keine ältere Bearbeitung zurück. Eine zuvor laufende Textbearbeitung
  wird vor dem Umbenennen als eigener Schritt übernommen.
  Klicks und Textauswahl im Namensfeld lösen kein Verschieben der Ebene aus.
- Text per Doppelklick, Enter auf ausgewähltem Text oder „Text ändern“ direkt
  auf der Arbeitsfläche nachbearbeiten. Enter übernimmt, Shift+Enter fügt eine
  Zeile ein, Esc verwirft einschließlich einer noch eingetippten Schriftgröße.
  „Übernehmen“ berücksichtigt auch Größenwerte ohne vorheriges Enter.
  Während der Eingabe nehmen Strg+Z und der Rückgängig-Button einzelne Schreib-,
  Lösch- und Stiländerungen zurück; Strg+Shift+Z oder Strg+Y stellt sie wieder her.
  Einfügen und das Ersetzen markierter Passagen bilden jeweils einen Schritt.
  Cursor und Auswahl bleiben dabei erhalten, auch bei Umlauten und Emoji.
  Die Textbearbeitung bleibt geöffnet; ältere Dokumentänderungen werden in dieser
  Zeit nicht zurückgenommen. Erst beim Übernehmen wird die gesamte Bearbeitung
  zu einem gemeinsamen Undo-Schritt im Dokument. Esc verwirft die Sitzung und
  bewahrt eine zuvor vorhandene Wiederholen-Historie des Dokuments.
  Beim Wechsel
  des Werkzeugs, Exportieren oder Schließen wird die laufende Eingabe übernommen;
  leere Eingaben werden verworfen. Text skaliert proportional
  über seine Höhe; die Breite ergibt sich aus Text und Schriftgröße.
- Alle Änderungen einschließlich Sichtbarkeit, Reihenfolge und Entfernen sind
  rückgängig machbar. Ein kompletter Ziehvorgang ist ein einzelner Undo-Schritt.

**Projekt (.untushot):** enthält das Original-PNG, alle Objekte, Sichtbarkeit,
Reihenfolge, Ebenennamen, Monitormetadaten und den Exportbereich. Beim erneuten Öffnen bleiben
sie bearbeitbar. Die Undo-Historie wird nicht mitgespeichert. Projektdateien werden
atomar geschrieben. JSON-Format mit eingebettetem PNG; PNG/JPEG lassen sich weiterhin
normal öffnen. Neue Projekte und Sitzungen verwenden Formatversion 4 für
Bildebenen mit editierbaren Rändern und Schatten. UntuShot liest weiterhin
Versionen 1–3; ältere Programmversionen können Dateien der Version 4 nicht öffnen.

**PNG / Zwischenablage:** enthalten nur das fertige, beschnittene Bild mit den
sichtbaren Ebenen. Projektdateien enthalten auch bei Schwärzungen das ursprüngliche,
ungeschwärzte Bild. Zum Teilen geschwärzter Inhalte den PNG-Export verwenden.

## Automatische Sitzungswiederherstellung

Geänderte Dokumente werden ungefähr einmal pro Sekunde als `.untushot`-Sitzung
atomar gesichert. PNG-Komprimierung und Dateischreiben laufen im Hintergrund.
Beim Schließen des Fensters wartet UntuShot auf die letzte Sicherung. Im
Hintergrundmodus bleibt das Dokument geöffnet; nach dem Beenden wird es beim
nächsten Öffnen des Editors wiederhergestellt.
Undo-Historie, laufende Ziehgesten und noch nicht übernommene Texteingaben werden
nicht gespeichert. Beim normalen Schließen wird die Texteingabe zuerst übernommen. Die
Arbeitseinstellungen werden separat gespeichert (siehe unten).
Bei einem Absturz kann die Änderung seit der letzten fertigen Sicherung fehlen.

Die Dateien liegen in `$XDG_STATE_HOME/untushot/recovery`, standardmäßig
`~/.local/state/untushot/recovery`. Das Verzeichnis hat Rechte `0700`, die Dateien
`0600`. Die Sitzung ist während der Verwendung gesperrt. UntuShot verwendet pro
Desktop-Sitzung eine Hintergrundinstanz; spätere Aufrufe werden an sie weitergeleitet.
Beim Öffnen des Editors wird die neueste freie Sitzung geladen. Ein erster Aufruf
mit `--open` oder `--capture` beginnt eine neue Sitzung. `--demo` läuft unabhängig
und verwendet keine Autosicherung.

Sicherungen bleiben auch nach dem Schließen erhalten. Sie enthalten wie Projekte
**das vollständige Original einschließlich geschwärzter oder abgeschnittener Bereiche**.
Zum vollständigen Entfernen der Sicherungen UntuShot über das Tray beenden und die Dateien im
Recovery-Verzeichnis löschen. Eine neue Aufnahme ersetzt die aktuelle Sitzung erst
nach erfolgreicher Aufnahme; separate Projekte sind weiterhin die dauerhafte Ablage.

## Ubuntu-Design und Akzentfarbe

Alle Aktionssymbole verwenden ein gemeinsames 20-px-Raster mit 16-px-Glyphen.
Icon-Buttons zentrieren ihren Inhalt auch in gestreckten Ebenenleisten;
Dateimenü und Werkzeugliste verwenden durchgehende Icon- und Textspalten.
Toolbar-Abstände, Tastenkürzelspalte und Stilfelder sind einheitlich ausgerichtet.

UntuShot verwendet die native GTK-Fensterleiste, Systemschrift, Bedienelemente und
symbolischen Theme-Icons. Unter Ubuntu bestimmen die Einstellungen in
„Darstellung“ das Yaru-Theme, den hellen/dunklen Stil und die Akzentfarbe.
UntuShot erzwingt kein eigenes dunkles Farbschema. Themewechsel werden im
laufenden Fenster übernommen; es gibt dafür keine zweite Farbeinstellung in UntuShot.
Auch Auswahlrahmen, Pixellupenrand und Monitorübersicht verwenden die Theme-Akzentfarbe.
Zeichenfarben und die Bilddaten bleiben unabhängig davon unverändert.

Die Werkzeuge sind nach Auswahl, Zeichnen und Hervorheben gruppiert. Nur das aktive
Werkzeug trägt eine zurückhaltende Akzentfläche. Farbe, Strichstärke und Schriftgröße
haben eigene beschriftete Felder. Die Ebenenliste zeigt im leeren Zustand eine kurze
Orientierung; Eigenschaften und Ebenenaktionen erscheinen erst bei einer Auswahl.
Die feste Inspectorbreite verhindert, dass die Arbeitsfläche dabei zur Seite springt.
Die Ebenenoptionen bleiben unten vollständig sichtbar und erhalten ihre benötigte
Höhe. Die Ebenenliste darüber füllt den verbleibenden Platz und scrollt unabhängig.
Beim Aufklappen zusätzlicher Optionen wird die Liste entsprechend kleiner.

Auf der Aufnahme-Seite stehen Quelle und Monitorübersicht in einer gemeinsamen
Karte. Der Hauptbutton nennt die gewählte Aktion und gegebenenfalls den Timer,
z. B. „Bereich auswählen · 3 s“. Editoraktionen sind dort ausgeblendet. Der
Vorschaurahmen auf der Arbeitsfläche ist nur eine UI-Hilfe und wird nicht exportiert.

## Arbeitseinstellungen und Speicherzustände

UntuShot merkt sich das zuletzt verwendete Werkzeug, Farbe, Strichstärke,
Schriftgröße, ein-/ausgeklappte Seitenleisten, den PNG-Exportordner und die
Monitor-Vorauswahl. Eine Monitorwahl wird nur übernommen, wenn Name, Geometrie
und Skalierung eindeutig zur aktuellen Anordnung passen. Sonst ist wieder der
gesamte Desktop voreingestellt; beim Portal erfolgt die Bestätigung im Desktopdialog.
Explizite CLI-Optionen `--monitor` und `--output` haben Vorrang.

Die Einstellungen werden nach Änderungen ungefähr einmal pro Sekunde und beim
Schließen atomar nach `$XDG_CONFIG_HOME/untushot/preferences.json` geschrieben,
standardmäßig `~/.config/untushot/preferences.json`. `--demo` liest und schreibt
keine persönlichen Einstellungen. Zoom und Bildverschiebung gelten nur für die
aktuelle Ansicht und verändern weder Ausschnitt noch Annotationen.

Die kompakte Statusleiste zeigt unten rechts den Projektstatus neben „Projekt speichern“.
Ein Klick darauf öffnet „Dein Arbeitsstand“ mit drei unabhängigen Zuständen:

- **Automatisch gesichert:** erst nach bestätigtem Schreiben der aktuellen Version.
  Währenddessen steht dort „Sicherung ausstehend“, bei Fehlern eine Fehlermeldung.
- **Projekt gespeichert / geändert:** bezieht sich auf die bearbeitbare Projektdatei.
  Undo zum gespeicherten Zustand stellt auch diese Anzeige wieder her.
- **PNG aktuell / enthält älteren Stand:** zeigt, ob seit dem Export weitere
  Änderungen erfolgt sind. Kopieren und PNG-Export speichern kein bearbeitbares Projekt.

Der Arbeitsstand trennt **Bilddatei zum Teilen**, **Projekt zum Weiterbearbeiten**
und **Beim nächsten Start**. Er zeigt die gespeicherten Dateinamen; vollständige
Pfade sind als Tooltip verfügbar. Lange Namen werden innerhalb des Popovers
umgebrochen. Nach einem aktuellen PNG-Export meldet der Fenstertitel „Bild gespeichert“;
nach weiteren Änderungen „Bearbeitet“.

Ein noch nicht übernommener Text wird zusätzlich als „Text in Bearbeitung“ angezeigt.
Beim Umbenennen einer Ebene zeigt die Projektanzeige entsprechend „Name in Bearbeitung“.
Speichern, Exportieren und Kopieren übernehmen zuerst offene Eingaben, auch eine
gerade eingetippte Schriftgröße ohne vorheriges Enter. „PNG speichern unter …“
rendert den aktuellen Stand erst nach Bestätigung des Dateiziels.
Beim Öffnen wird die gewählte Datei zuerst auf Lesbarkeit geprüft; ein Fehler
lässt das aktuelle Dokument und seine Speicherziele bestehen.
Fensterschließen oder Alt+F4 blendet den Editor bei aktivem Tray aus. Zum Beenden
„UntuShot beenden“ im Tray oder `untushot --quit` verwenden. Ohne Tray-Anbindung
beendet das Fensterschließen die Anwendung. Esc beendet sie nicht.

## UX/UI-Roadmap — Stand 0.8.1

Die Tabelle unterscheidet implementierte Funktionen von noch ausstehender
Praxisabnahme. Barrierefreiheit ist auf Wunsch nicht Teil dieser Roadmap.

| Bereich | Stand | Nächster überprüfbarer Abschluss |
| --- | --- | --- |
| Aufnahme → Auswahl → Editor, Timer | Implementiert; 0.7.6 stoppt bei geänderter Monitoranordnung | Gesamtablauf auf mehreren physischen Monitoren prüfen |
| Editierbare Markierungen, Undo/Redo, Projekte | 30 Ebenen auf einem 4K-Testbild: 140 Änderungen einschließlich 10 Textsitzungen, 100 Schritte Undo/Redo, PNG, Zwischenablage, Projekt und Wiederherstellung automatisiert geprüft | Längere Bearbeitungssitzung mit eigenen Bildern und echter Desktop-Skalierung |
| Ubuntu-Stil, Systemakzent, Icons, Stärkevorschau | Implementiert; helle/dunkle Darstellung geprüft | Prüfung mit echter Desktop-Skalierung und langen Beschriftungen erweitern |
| Verständlicher Speicherfluss | Offene Eingaben vor Speichern/Kopieren übernehmen; PNG-Pixelvergleich, Speichern unter, Dateifehler, Bildwechsel und Tray-Sicherung GTK-getestet | Alltagsnutzung mit bestehenden Bild- und Projektdateien |
| Kleine Fenster | Verlauf unter der Bildfläche; volle Höhe für Werkzeuge und Ebenen. 30 Ebenen, sichtbare Bildeffekte und 60 Verlaufsbilder gemeinsam bei 700 Pixeln Fensterhöhe geprüft; sichtbare Auswahl bleibt beim Verkleinern erreichbar | Praxisprüfung mit echter Desktop-Skalierung |
| Direkte Eigenschaftenbearbeitung | Position, Größe, Randstärke und Schatten live; Bearbeitung pro Feld als ein Undo-Schritt, Speichern und Wiederöffnen GTK-getestet | Längere Alltagssitzungen und echte Desktop-Skalierung prüfen |
| Präzises Platzieren | Hilfslinien und optionales Einrasten beim Verschieben; Bildausschnitt, sichtbare Ebenen und Gruppen; Alt-Ausnahme | Praxisprüfung bei sehr vielen Ebenen und gemischter Desktop-Skalierung |
| Aktionen am Objekt | Rechtsklick: Duplizieren, Reihenfolge, Löschen und Text bearbeiten; Mehrfachauswahl bleibt erhalten | Alltagstest mit überlappenden und vielen Ebenen |
| Tray, Tastenkürzel, „Öffnen mit“, Bildverlauf | Implementiert | Durchgängige Praxisabnahme: Hotkey → Aufnahme → Bearbeitung → Speichern → Bildwechsel → Neustart |
| Multimonitor / gemischte DPI | 0.7.6: lokale 4K-/Hochformat-Geometrie geprüft; Monitorwechsel, Timer-Abbruch und Systemauswahl abgesichert | Echte Aufnahme/Hotplug-Abnahme; verschiedene Skalierungen, Abbruch und Wiederholung |
| Vollständige Webseiten | 0.7.7: Übernahme eines im aktuellen Browser kopierten Seitenbilds mit scrollbarer Vorschau; Aufnahme im Browser manuell | Automatische Tab-Anbindung mit Anmeldung, Formularzustand und nachladenden Inhalten |
| Serienaufnahmen | Offen, nach Webseiten | Intervall und Anzahl → Aufnahmefolge → Bildauswahl |
| GTK4-Migration | Langfristig offen | GTK3-Rückports durch eine gewartete Plattform ablösen; bestehende Abläufe erhalten |

Die vier geplanten Bedienungspakete sind umgesetzt: kompakte Eigenschaften,
direkte Vorschau, präzises Platzieren und Objektaktionen. Der anschließende
30-Ebenen-Test hat das automatische Anzeigen einer ausgewählten Ebene,
stabile Zeilen bei kleinen Änderungen und das Erhalten unfertiger Ebenennamen
abgesichert. Beim Verschieben werden unveränderte Bildteile zwischengespeichert;
Text wird weiterhin direkt gezeichnet, damit seine Kantenglättung erhalten bleibt.
Als Nächstes folgt die vollständige Praxisabnahme einschließlich realer Multimonitor-Skalierung.
Danach werden die 20 UX/UI-Kriterien erneut bewertet.
Automatische Webseitenaufnahme und Serienaufnahmen bleiben nachgelagert.
Screenshots mit synthetischen Testbildern ersetzen die Hardware- und Alltagstests nicht.

## Flow-Bilder aus mehreren Screenshots

Ab 0.8.0: unten im Verlauf einen Screenshot **rechts anklicken → Rechts an
aktuelles Bild anfügen**. Alternativ im Dateimenü **Bild rechts anfügen …**
(`Strg+I`) eine PNG- oder JPEG-Datei wählen. Ein normaler Linksklick im Verlauf
öffnet weiterhin das gewählte Bild als neues Dokument.

Die Arbeitsfläche wächst nach rechts, mit 24 Pixeln transparentem Abstand.
Die Auflösung des hinzugefügten Bildes bleibt erhalten. Auch das Ausgangsbild
wird dabei zu einer Bildebene; vorhandene Pfeile, Texte und Formen bleiben
separate, bearbeitbare Ebenen. Ein bestehender Ausschnitt wird übernommen;
abgeschnittene Inhalte erscheinen beim Erweitern nicht wieder.

- Bildebenen mit `V` verschieben und am Griff rechts unten proportional skalieren.
  Auch bei Eingabe von Breite oder Höhe bleibt das Seitenverhältnis erhalten.
- In der Ebenenliste umbenennen, ausblenden, duplizieren, umsortieren oder löschen.
- `Strg+Z` nimmt das gesamte Anfügen einschließlich der Arbeitsflächengröße zurück.
- PNG exportiert die fertige Komposition. Eine `.untushot`-Projektdatei erhält
  alle Bildebenen und Markierungen; Bilddateien werden eingebettet und müssen
  beim erneuten Öffnen nicht mehr am ursprünglichen Ort liegen. Die Autosicherung
  enthält ebenfalls die Bildebenen.

Neue Projekte verwenden Formatversion 4; alte Projekte bleiben lesbar.
Ältere UntuShot-Versionen können neue Projekte nicht öffnen. Die Arbeitsfläche
ist auf 64 Megapixel und 32.767 Pixel je Seite begrenzt. Erweiterungen nach
unten, frei einstellbare Abstände und automatische Flow-Verbindungen sind noch
nicht enthalten.

## Webseiten aus der aktuellen Browsersitzung

Ab 0.7.7 übernimmt UntuShot ein im Browser kopiertes Seitenbild. Die Aufnahme
löst du in deiner bestehenden Browsersitzung aus; UntuShot übernimmt anschließend
das fertige Bild. Die automatische Steuerung oder Aufnahme des aktuellen Tabs
ist noch offen. UntuShot liest keine Browserprofile oder Anmeldedaten.

1. In Firefox einen Rechtsklick auf eine freie Stelle der Seite machen und
   **Bildschirmfoto aufnehmen** wählen. Die gesamte Seite auswählen und kopieren.
   Das Vorgehen beschreibt die [Firefox-Hilfe](https://support.mozilla.org/en-US/kb/take-screenshots-firefox).
2. In UntuShot **Neue Aufnahme → Webseite übernehmen … → Bild einfügen** wählen.
   Für ein bereits kopiertes Bild **Bild als neues Dokument …** im Dateimenü
   oder **Strg+Umschalt+V** wählen. **Strg+V** fügt im geöffneten Editor
   stattdessen eine neue Bildebene ein.
3. Die Vorschau bis zum Seitenende scrollen. Standardmäßig passt sie in die Breite;
   **100 %** zeigt die Originalgröße. Nach dem Einfügen klappt die Anleitung ein.
4. **Im Editor öffnen** übernimmt das vollständige Originalbild. Vor dem Ersetzen
   einer geänderten Bearbeitung erscheint die übliche Speicherrückfrage. Abbrechen
   behält das bisherige Dokument. Anschließend stehen alle Werkzeuge und Ebenen bereit.

Der Import unterstützt PNG und JPEG aus der Zwischenablage. Er liest sie nur auf
explizite Anforderung, dekodiert im Hintergrund und übernimmt keine verspäteten
Ergebnisse nach Abbruch. Die Vorschau verkleinert ausschließlich die Anzeige;
Export und Projekt behalten die volle Auflösung. Neue Importe erhalten kein
automatisches Überschreibziel.

Die Importgrenzen sind 32 MiB kodierte Bilddaten, 40 Millionen Pixel und 32.767 Pixel
je Seite. Größere Seiten müssen in Abschnitten aufgenommen werden. Nachladende
Inhalte vor der Aufnahme im Browser laden; verschachtelte Scrollbereiche und
endlos nachladende Seiten sind mit einem einzelnen Seitenbild nicht vollständig
abgedeckt. Die Vorschau hilft, das tatsächlich erfasste Seitenende zu prüfen.

Eine automatische Browser-Anbindung und Serienaufnahmen bleiben weitere
Arbeitspakete. Serien benötigen Intervall, Anzahl und eine Auswahlübersicht;
allgemeines Scrollen fremder Wayland-Apps zusätzlich eine eigene Integration.

## Installation und Screenshot-Taste

```sh
make install
```

Auf Ubuntu/GNOME **als normaler Benutzer ohne `sudo`** in der grafischen Sitzung
ausführen. Der Installer baut das Release und richtet Folgendes ein:

- Startbefehl `~/.local/bin/untushot`, Binary `~/.local/libexec/untushot`,
  Desktop-Eintrag und Icons unter `~/.local/share`.
- **Strg+Shift+X** startet direkt eine Bereichsaufnahme. Nach der Portal-Freigabe
  folgt die Bereichsauswahl und anschließend der Editor mit Werkzeugen und Ebenen.
- **Symbol oben rechts:** „Bereich auswählen“, „Bereich auswählen · 3 Sekunden“,
  „Ganzer Bildschirm“, „Systemauswahl (Monitor / Fenster)“, „Editor öffnen“,
  „Aufnahme abbrechen“ und „UntuShot beenden“. Der Status zeigt Bereitschaft oder
  Countdown; während einer Aufnahme sind weitere Aufnahmen gesperrt.
- **Autostart beim Anmelden** und sofortiger Start im Hintergrund, ohne Editorfenster.
  GNOMEs Ubuntu-AppIndicators-Erweiterung muss aktiviert sein. Die Bibliothek
  `libayatana-appindicator3.so.1` wird zur Laufzeit geladen; ohne sie funktioniert
  weiterhin der Editor, der Hintergrundmodus meldet den fehlenden Tray-Support.

Eine bereits belegte Kombination Strg+Shift+X, beispielsweise `flameshot gui`, wird
für UntuShot übernommen. Die vorherigen Werte werden in
`~/.config/untushot/desktop-integration.json` gesichert. Erneutes Installieren erzeugt
keine doppelten Tastenkürzel. `make uninstall` beendet die Hintergrundinstanz,
entfernt die installierten Dateien und stellt die frühere Belegung sowie einen
vorherigen Autostart-Eintrag wieder her, sofern du sie seitdem nicht geändert hast.
Projekte, Autosicherungen und persönliche Einstellungen bleiben erhalten.

Der Tastaturbefehl verwendet einen absoluten Pfad und funktioniert unabhängig vom
`PATH`. Für Aufrufe im Terminal sollte `~/.local/bin` im `PATH` liegen.

```sh
untushot --background    # Nur Tray starten; keine zweite Instanz
untushot                 # Editor bzw. Aufnahme-Seite öffnen
untushot --cancel        # Laufende Aufnahme abbrechen
untushot --status        # JSON: running, tray_connected, capturing, pid
untushot --quit          # Sitzung sichern und Hintergrundinstanz beenden
make desktop-install    # GNOME-Anbindung nachträglich einrichten / neu starten
make install DESKTOP_INTEGRATION=off   # Nur Dateien installieren
make install PREFIX=/usr DESTDIR=/tmp/untushot-package  # Paket-Staging, keine Desktopänderung
```

Der Autostart liegt unter `~/.config/autostart/io.github.untushot.UntuShot.desktop`,
das Hintergrundprotokoll unter `~/.local/state/untushot/background.log`.
Die entsprechenden XDG-Verzeichnisvariablen werden berücksichtigt.
Auf anderen Desktops wird die globale Kombination über deren Einstellungen
registriert, beispielsweise in Sway:

```text
bindsym Ctrl+Shift+x exec ~/.local/bin/untushot --capture region --backend grim
```

Unter GNOME/Wayland bleibt die Freigabe des Screenshot-Portals erforderlich.
„Ganzer Bildschirm“ übernimmt das vom Portal gelieferte Bild; die Systemauswahl
richtet sich nach dem Portal-Backend. Ein Timer verzögert die **Anforderung** der
Aufnahme, nicht die anschließende Freigabe im Systemdialog.
Upload-Dienst, Rotation und Raster-Masken sind noch nicht enthalten. Der PNG-Dialog
erzeugt immer PNG, auch bei abweichendem Dateisuffix; der Projekt-Dialog immer ein Projekt.

## Entwickeln und prüfen

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
/usr/bin/python3 -m unittest discover -s packaging/tests
```

Die normalen Tests benötigen keinen Display-Server. Sie prüfen unter anderem
Multimonitor-Geometrie, negative Ursprünge, Mixed-DPI, monitorübergreifende Ausschnitte,
Koordinatenumrechnung, Undo/Redo, deckende Schwärzung, PNG-Roundtrip, Ebenenänderungen,
den verlustfreien Projekt-Roundtrip, Auswahlgriffe, Gruppenedits sowie Wiederherstellung
und Sperren zwischen gleichzeitig laufenden Sitzungen.

Der Bildimport prüft zusätzlich lange Bilder einschließlich Seitenende und Alpha,
JPEG, ungültige Daten und zu große Bildheader vor dem Dekodieren. Der echte
GTK-Zwischenablage-Transport lässt sich auf einem privaten Display prüfen:

```sh
make test-clipboard
```

Dieser Test benötigt `broadwayd` (GTK3), erzeugt ein isoliertes temporäres Display
und prüft Bildübernahme sowie eine Zwischenablage ohne Bild. Er berührt die echte
Desktop-Zwischenablage nicht und benötigt keine Browser-Erweiterung.

In einer grafischen Sitzung kann zusätzlich der GTK-Integrationstest laufen:

```sh
cargo test --locked gtk_smoke -- --ignored --test-threads=1
```

Die Editorabläufe lassen sich auch ohne physische Monitorinformationen auf einem
privaten GTK-Testdisplay prüfen:

```sh
cargo test --locked gtk_editor_workflow -- --ignored --test-threads=1
```

Dieser Test prüft die sichtbaren Export- und Projektaktionen, Projektstatus nach
Bearbeitung und Undo, die kompakte Fenstergröße, aufklappbare Eigenschaften sowie
das Ausblenden und automatische Wiedererscheinen des Screenshot-Verlaufs.
Die acht Objektgriffe werden bei unterschiedlichen Zoomstufen und verschobener
Ansicht einschließlich Randtreffern, Mauszeigern, Vorschau, Esc und Undo/Redo geprüft.
Die Ebenenzeilen werden zusätzlich auf direkte Sichtbarkeitsschalter, erhaltene
Mehrfachauswahl, Vorschau-Cache und gespeicherte Sichtbarkeit geprüft.

Beide Editorprüfungen lassen sich reproduzierbar auf einem privaten GTK-Display starten:

```sh
make test-ux
```

Die Prüfungen verwenden temporäre XDG-Daten-, Cache- und Einstellungsverzeichnisse
sowie flüchtige Desktop-Einstellungen. Der Speicherablauf vergleicht PNG und
Zwischenablage pixelgenau mit dem Dokument, auch bei noch eingetippter Schriftgröße.
Er prüft außerdem das Übernehmen beim Ausblenden in den Tray und den Erhalt des
aktuellen Projekts bei fehlenden oder beschädigten Dateien.

Die Textprüfung umfasst Übernehmen ohne vorheriges Enter im Größenfeld,
lokales Undo/Redo mit Cursor und rückwärts markierter Auswahl, Unicode,
mehrzeiliges Einfügen aus der GTK-Zwischenablage, Löschen und neue Undo-Zweige.
Auch Schriftgröße/Farbe, der gemeinsame Dokument-Schritt nach Übernehmen,
die erhaltene Dokument-Historie nach Abbrechen sowie Escape mit
offener Größenänderung sowie leere und unveränderte Entwürfe. Ungültige Schrift-
und Stärkewerte erhalten den letzten gültigen Wert; Stärkevorlagen bleiben
rückgängig machbar. Tastaturereignisse und Toolbar-Aktionen werden gemeinsam geprüft.
Weitere GTK-Prüfungen decken offene Größenwerte beim Ebenen-/Werkzeugwechsel,
Duplizieren und Undo ab. Strg+O wird mit fokussiertem Text-, Namens- und Größenfeld
sowie anschließendem Abbrechen der Dateiauswahl geprüft.

Beim Umbenennen prüfen die GTK-Tests offene Namen mit Undo/Redo, unveränderte
und veraltete Entwürfe sowie Escape mit vorhandener Redo-Historie. Das gilt auch
für frisch geöffnete Projekte ohne Undo-Historie. Textänderung und anschließendes
Umbenennen bleiben zwei getrennte Schritte; die Ebene bleibt bei Namens-Undo erhalten.

Der Verlaufstest verwendet 60 Bilder mit langen Dateinamen. Er prüft stabile
Kacheln, erhaltene Vorschauen und die sichtbare Scrollposition beim Hinzufügen,
Ändern, Löschen sowie Ein- und Ausklappen. Ein geändertes Bild muss neue
Vorschaupixel erhalten; ein unverändertes behält seine bestehende Vorschau.
Der gemeinsame Layouttest enthält außerdem 30 Bildebenen mit Rand und Schatten.
Bei 700 Pixeln Fensterhöhe bleiben die Effekte erreichbar; Ein- und Ausklappen
des Verlaufs verändert die Höhe der Ebenenliste nicht. Geprüft werden auch das
Umhängen der Leiste zwischen ihren beiden Bereichen und der Erhalt einer
sichtbaren Ebenenauswahl beim Verkleinern. `target/gtk-history-many.png` zeigt
den Verlauf zusammen mit den Ebenen und Effekten im kompakten Editor.

Der zusätzliche Test verwendet ein synthetisches 3840 × 2160-Bild mit 30 Ebenen
(Bilder, Text, Rechtecke), langen Namen sowie Rand und Schatten. Er prüft das
automatische Scrollen zur Auswahl, die erhaltene manuelle Scrollposition,
stabile Ebenenzeilen und unfertige Namenseingaben. Eine längere Folge von
130 Verschiebungen und 10 Textsitzungen überschreitet die Undo-Grenze. Die jeweils
letzten 100 Dokumentänderungen werden vollständig zurückgenommen und wiederholt;
nach jedem Schritt werden sämtliche Ebenen mit dem erwarteten Stand verglichen.
Lokales Text-Undo/Redo bleibt dabei auf die offene Textsitzung begrenzt.
Anschließend werden Projekt, PNG, Zwischenablage und Autosicherung mit allen
Ebenen bzw. exportierten Pixeln verglichen. Zusätzliche Dokumenttests prüfen,
dass verworfene Redo-Bilder und beim nächsten Schritt ablaufende Undo-Bilder
keine neuen Bildimporte blockieren. Noch wiederherstellbare Bilder bleiben erhalten;
fehlgeschlagene Importe verändern den vorhandenen Verlauf nicht.
`target/gtk-many-layers.png` zeigt das Ergebnis; `target/ux-audit.json` enthält
Median und 95. Perzentil für kleine Änderungen und gezeichnete Verschiebeschritte.
Diese CPU-Zeiten im Testprozess sind keine Messung der Desktop-Bildrate.
Zusätzliche Renderingtests vergleichen überlappende und ausgeschnittene Ebenen
bei Zoom und 1×/2×-Skalierung. Der zusätzliche Cache ist auf 64 MiB begrenzt;
bei größeren Anforderungen wird direkt gezeichnet.

Der vollständige Test (`gtk_smoke`) öffnet kurz den Demo-Editor und prüft Monitorwahl, Werkzeugwechsel,
Objektverschiebung, Mehrfachauswahl, Duplizieren, Ausrichten, Umbenennen,
Stil/Sichtbarkeit, Undo/Redo, Auswahlgriffe, Seitenleisten, kleine Fenster,
Aufnahme-Seite, Countdown-Abbruch und die Sicherung beim Schließen. Zusätzlich
prüft er Enter/Esc, direkte Textbearbeitung, unabhängige Schriftgrößen, Zoom-Anker,
Verschieben der Ansicht, gespeicherte Einstellungen in temporären Verzeichnissen
und getrennte Speicheranzeigen einschließlich Undo zum gespeicherten Zustand.
Er prüft außerdem das Sichern/Ausblenden im Hintergrundmodus, Wiederöffnen und
das Abbrechen einer direkten Bereichsaufnahme vor Ablauf ihres Timers.
Er prüft außerdem die Browser-Importvorschau, Breitenanpassung/Originalgröße,
Bildwechsel mit Abbruch und verspätete Decoder-Ergebnisse. Dabei verwendet er
synthetische Bilddaten und liest oder verändert keine Desktop-Zwischenablage.
Er rendert eigene Widgets unter anderem nach `target/gtk-smoke.png`,
`target/gtk-capture.png` und `target/gtk-web-import.png`.
Er nimmt den echten Desktop nicht auf. Die Bereichsauswahl wird über denselben
Einstieg wie ein echtes Aufnahmeergebnis geprüft, einschließlich 100 Ziehschritten,
Bestätigung vor dem Export und Monitorbegrenzung. Ein zusätzlicher Rendertest
vergleicht die Vorschau pixelgenau bei Displayskalierung 1× und 2×.
Der GTK-Test prüft außerdem den Wechsel zwischen dunklem Yaru mit Salbeigrün und
hellem Yaru mit Blau im selben Fenster und rendert Vorschauen nach
`target/gtk-theme-dark.png` und `target/gtk-theme-light.png`. Er ändert nur das
Theme seines eigenen Prozesses, keine GNOME-Systemeinstellungen. Zusätzlich prüft
er den Leerzustand der Ebenenliste, eine unveränderte Arbeitsflächenbreite beim
Auswählen, den Speicherstatus-Popover und die Beschriftungen für Modus und Timer.

Die reine Bilddarstellung einer synthetischen Multimonitor-Aufnahme lässt sich
separat messen (keine Bildschirmaufnahme):

```sh
cargo test --release --locked large_desktop_render_timing -- --ignored --nocapture
```

### Start aus einer VS-Code-Snap-Umgebung

Einige IDE-Terminals vererben GTK- und GSettings-Pfade aus ihrem Snap. Falls der
Start mit einem fehlenden `antialiasing`-Schema-Key abbricht, in einem normalen
Systemterminal starten. Der durch `make install` erzeugte Starter bereinigt diese
Pfade automatisch. Für den direkten Aufruf des Build-Binarys:

```sh
env -u GTK_PATH -u GIO_MODULE_DIR \
  GSETTINGS_SCHEMA_DIR=/usr/share/glib-2.0/schemas \
  XDG_DATA_DIRS=/usr/local/share:/usr/share \
  GDK_BACKEND=wayland ./target/release/untushot
```

## Quellen und Architektur

- [`src/capture.rs`](../src/capture.rs): asynchroner Portal-Request mit Timeout und
  Subscription vor dem Aufruf; `grim` läuft auf einem Hintergrundthread.
- [`src/geometry.rs`](../src/geometry.rs): Desktop-Layout und Pixelkoordinaten.
- [`src/document.rs`](../src/document.rs): Dokument, Historie, gemeinsamer Renderer.
- [`src/pixels.rs`](../src/pixels.rs): PNG/JPEG-Decoder und PNG-Encoder in Rust.
- [`src/preview.rs`](../src/preview.rs): Vorschau-Cache in Fensterauflösung für flüssiges Auswählen.
- [`src/ui.rs`](../src/ui.rs): GTK-Editor, Objektinteraktion, Zoom, atomare Exporte.
- [`src/web_import.rs`](../src/web_import.rs): expliziter Bildimport mit scrollbarer Vorschau und asynchronem Dekodieren.
- [`src/runtime.rs`](../src/runtime.rs): gemeinsame Instanz, weitergeleitete CLI-Befehle und Lebenszyklus.
- [`src/tray.rs`](../src/tray.rs): native Ayatana-AppIndicator-Anbindung und Statusmenü.
- [`packaging/install.py`](../packaging/install.py): Installation, GNOME-Tastenkürzel, Autostart und Rückbau.
- [`src/setup.rs`](../src/setup.rs): Aufnahme-Seite, Quellen und Timer.
- [`src/selection.rs`](../src/selection.rs): pixelgenaue Auswahl, acht Griffe und Bewegung.
- [`src/monitor_map.rs`](../src/monitor_map.rs): anklickbare Monitoranordnung.
- [`src/recovery.rs`](../src/recovery.rs): Hintergrundsicherung mit Schreibbestätigung, Instanzsperren und Wiederherstellung.
- [`src/preferences.rs`](../src/preferences.rs): atomare Arbeitseinstellungen und sichere Monitorzuordnung.
- [`src/history.rs`](../src/history.rs): Screenshot-Ordner, asynchrone Vorschaubilder und begrenzter Cache.
- [`src/theme.rs`](../src/theme.rs): Systemfarben für die selbst gezeichneten Auswahl- und Monitoransichten.
- [`src/icons.rs`](../src/icons.rs): native symbolische Icons mit Vektor-Fallbacks für Zeichenwerkzeuge.
- [XDG Screenshot Portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Screenshot.html)
- [Ayatana AppIndicator API](https://github.com/AyatanaIndicators/libayatana-appindicator/blob/main/src/app-indicator.h)
- [grim](https://gitlab.freedesktop.org/emersion/grim)
- [GTK-Monitor-API](https://docs.gtk.org/gdk3/method.Display.get_monitor.html)

Lizenz: MIT. Eigenständige Implementierung, kein offizielles Flameshot-Projekt.
