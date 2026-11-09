# Continuità delle sessioni: note per la prossima implementazione

Data: 2026-11-08. La prima implementazione copre tutte le sei aree.
Stato delle verifiche, utilizzo e limiti sono registrati in
[session-continuity-implementation.md](session-continuity-implementation.md).
Il backlog seguente conserva i criteri originali, inclusi gli sviluppi futuri.

## Obiettivo e punto di partenza

L'utente vuole trasferire il lavoro a una nuova chat e avere compattazione
automatica. Ha chiesto ulteriori migliorie ispirate a Pi, oh-my-pi (interpretazione
di «omp») e Claude Code.

Il punto di partenza era `/handoff` nella TUI: produce un riepilogo
tramite una richiesta temporanea isolata e apre una nuova sessione con quel
riepilogo come messaggio iniziale. La sessione originale resta riprendibile.
La generazione usa gli scambi visibili utente/assistente, limitati a 64 KiB;
non include direttamente output grezzi dei tool, ragionamento o allegati.
Controlli su identità della richiesta, revisione del turno, bozze e input in coda
impediscono di trasferire uno stato superato o perdere nuovi input.

La compattazione automatica era già presente: non è stato aggiunto un secondo
motore. Il limite configurabile è `model_auto_compact_token_limit`, soggetto al
limite del modello (90% della finestra nel codice esaminato). Vedere
[config.md](config.md) per la documentazione aggiunta.

Punti di ingresso da rileggere prima di modificare il codice:

- `codex-rs/tui/src/app/recap.rs`: orchestrazione recap/handoff e nuova sessione.
- `codex-rs/tui/src/app/recap_history.rs`: selezione e limite della cronologia.
- `codex-rs/tui/src/temporary_structured_request.rs`: richiesta temporanea.
- `codex-rs/tui/src/app_backtrack.rs`: navigazione dei turni già esistente.
- `codex-rs/core/src/session/turn.rs`: compattazione automatica nei turni.
- `codex-rs/core/src/session/context_window.rs`: contabilità del contesto.
- `codex-rs/protocol/src/openai_models.rs`: limite di compattazione del modello.

Fork, backtracking, hook, subagenti e gestione degli input in coda sono già
fondamenta disponibili: ricostruire i flussi esistenti prima di aggiungerne altri.
Le modifiche `/handoff` sono locali e non risultavano committate al salvataggio
di queste note; verificarne lo stato senza sovrascriverle.

## Backlog ordinato

### 1. Handoff con stato strutturato persistente

Conservare obiettivo, vincoli, decisioni, attività aperte, file modificati,
verifiche e prossimi passi in uno stato persistente condivisibile dai client.
Collegare esplicitamente sessione sorgente e destinazione. Il testo di handoff
deve essere una vista di questo stato, evitando due fonti di verità.

Prima di scegliere schema e storage, ricostruire protocollo, eventi e persistenza
delle sessioni nel backend. Riutilizzare questi confini; non affidare lo stato
canonico esclusivamente alla TUI o a un file Markdown generato dal modello.
Separare fatti osservati e sintesi del modello, con provenienza e revisione.

Criteri di accettazione: stato ripristinabile dopo riavvio, compatibilità con
sessioni precedenti, nessuna perdita di bozze/input concorrenti, trasferimenti
falliti recuperabili e risultati obsoleti rifiutati. Il riepilogo deve ricordare
di verificare il workspace effettivo. Definire limiti e trattamento dei dati
sensibili prima di persistere output aggiuntivi.

### 2. Registro delle verifiche

Registrare comando, directory, risultato, momento e versione dello stato
verificato. Distinguere superato, fallito, non eseguito e obsoleto. Una modifica
successiva deve invalidare le verifiche interessate: iniziare con una regola
conservativa esplicita, senza inventare dipendenze precise tra test e file.
Il solo commit Git non identifica modifiche non committate o file non tracciati.

Acquisire gli esiti dal runtime degli strumenti; il modello può descriverli ma
non certificare autonomamente un test come superato. Riutilizzare il registro
nel riepilogo persistente e nella visualizzazione delle verifiche.

Criteri di accettazione: esiti ricostruibili, timeout/interruzioni distinti dal
successo, invalidazione dopo modifiche, assenza di falsi «test passati» dopo
handoff o compact. Considerare un controllo preliminare di spazio disco,
dipendenze e processi attivi prima di build costose, senza terminare processi o
cancellare dati automaticamente.

### 3. Ispettore del contesto

Mostrare il peso di istruzioni, cronologia, output degli strumenti e definizioni
MCP, oltre alla soglia di compattazione e alla capacità disponibile. Distinguere
conteggi osservati da stime. Riutilizzare la contabilità esistente evitando
numeri apparentemente precisi ma non riconciliabili con il runtime.

Valutare pochi vincoli esplicitamente fissabili dall'utente, con limiti di spazio
e regole chiare su cosa resta dopo il compact. Non duplicare indiscriminatamente
le istruzioni a ogni turno.

### 4. Diagnostica LSP dopo le modifiche

Integrare diagnostica, definizioni, riferimenti e rinomina semantica dove utili.
Eseguire la diagnostica dopo gruppi coerenti di modifiche, correlando i risultati
alla versione del documento e ignorando quelli obsoleti. I test/build restano
verifiche separate. Prima verificare gli eventuali supporti LSP già presenti:
la precedente ricerca era limitata all'area dei tool, non esaustiva del repository.

### 5. Cronologia navigabile e compact selettivo

Unificare la navigazione di fork, handoff e backtracking in una vista delle
diramazioni. Permettere di riassumere una fase conclusa preservando quella
attiva. Separare esplicitamente ripristino della conversazione e ripristino dei
file: gli effetti esterni dei comandi shell non sono automaticamente reversibili.
Preservare la cronologia originale e verificare compatibilità con resume/fork.

### 6. Modifiche protette dalla versione del file

Verificare che la versione di un file sia ancora quella letta prima di applicare
una modifica. In caso contrario rileggere e ricalcolare, evitando sovrascritture
silenziose di cambiamenti concorrenti. Prima valutare le garanzie già offerte
dagli strumenti di patch. Gli ancoraggi hashline di OMP sono una possibile
implementazione, da confrontare con alternative più semplici e misurare sui
modelli effettivamente utilizzati; non assumere i benchmark promozionali come
risultati validati per Codex.

## Sequenza suggerita per riprendere il lavoro

1. Leggere queste note, le istruzioni applicabili e lo stato Git corrente.
2. Ricostruire persistenza, protocollo e ciclo di vita dei turni per le priorità
   1–2; documentare la scelta minima coerente e le invarianti prima di implementare.
3. Aggiungere stato persistente e registro con test di resume, concorrenza,
   invalidazione e fallimento; collegarli al comando `/handoff` esistente.
4. Verificare la diff architetturale e documentare comportamento e limiti.
5. Affrontare separatamente le altre proposte in base alle priorità dell'utente.

## Verifiche già eseguite su `/handoff`

Questi sono risultati della sessione precedente, non nuove verifiche di queste
note e non una garanzia sul futuro stato del repository:

- `cargo check -p codex-tui --tests`: superato.
- Test TUI filtrati: `recap` 65 superati; `temporary_structured_request` 8;
  `thread_title` 28.
- Filtro `slash`: 233 superati e un fallimento di snapshot Daybreak non
  modificato, relativo a duplicazione di snapshot inline; lo stesso test isolato
  è passato. Non dichiarare l'intera suite verde.
- Formattazione dei file Rust modificati e `git diff --check`: superati.

Le build hanno esaurito lo spazio disco con cache incrementali. Per i test è
stato usato `CARGO_INCREMENTAL=0`, `RUST_MIN_STACK=8388608` e:

```sh
cargo test -p codex-tui --lib <filtro> --config 'profile.test.package.codex-tui.debug=0'
```

Il target Cargo era reindirizzato da lean-ctx alla sua cache. Controllare spazio
e configurazione effettivi prima di rilanciare build ampie. La CLI installata
non è stata aggiornata con il nuovo comando.

## Fonti consultate per le proposte

Fonti ufficiali consultate nella sessione del 2026-11-08; verificare nuovamente
le API quando si passa all'implementazione.

- [Pi: compaction e branch summaries](https://pi.dev/docs/latest/compaction):
  riepiloghi strutturati, decisioni, attività e tracciamento dei file.
- [Pi: sessioni](https://pi.dev/docs/latest/sessions) e
  [estensioni](https://pi.dev/docs/latest/extensions): navigazione e lifecycle.
- [oh-my-pi](https://github.com/can1357/oh-my-pi) e
  [configurazione LSP](https://github.com/can1357/oh-my-pi/blob/main/docs/lsp-config.md):
  diagnostica, operazioni semantiche e modifiche hashline.
- [Claude Code: checkpointing](https://code.claude.com/docs/en/checkpointing):
  rewind e riepiloghi selettivi, con distinzione tra codice e conversazione.
- [Claude Code: hooks](https://code.claude.com/docs/en/hooks-guide): integrazione
  con eventi del ciclo di vita e continuità dopo compattazione.
- [Claude Code: costi e utilizzo](https://code.claude.com/docs/en/costs):
  osservabilità dell'utilizzo e della cache.

Queste sono ispirazioni di prodotto: schema persistente, registro delle verifiche
e criteri di accettazione sopra sono proposte per questo repository, non una
dichiarazione che gli altri strumenti implementino esattamente lo stesso modello.
