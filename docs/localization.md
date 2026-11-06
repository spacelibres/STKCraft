# Translating the interface

STKCraft's interface is written in English in the source and translated by catalogs. The catalogs are
the ArtCraft family's format — the same one `storytold/craft-i18n` holds and the other Craft apps
read — so a translation written here can be contributed there, and a rule that disagrees with the
shared checker is a bug here rather than a local preference. See
[standards.md](standards.md) for the rule that governs all of this.

## The one rule

**Every user-visible string goes through a macro:**

```rust
stkcraft_i18n::tr!(translations, locale, "Save")
stkcraft_i18n::trc!(translations, locale, "menu", "Open Table…")   // a context, when one English has two meanings
stkcraft_i18n::trn!(translations, locale, "{n} satellite", "{n} satellites", count)   // a count
```

A string built at run time is a string no catalog can be checked against, and a string formatted
with `format!` before it reaches the macro is a translation of the format string rather than of the
message. Both are why the macros take literals.

`cargo xtask i18n` reads those literals out of the source. It is the only mechanism: there is no
extractor file to update, and a string that is not in a macro is invisible to it. `cargo xtask ci`
runs the check, so an untranslatable string is caught the same day it lands rather than when someone
translates the app.

**What is not translated:** command ids, satellite and station names, the epoch a time is printed in,
and anything the engine computes. The rule the family uses is "interface chrome, not document text":
a scenario's name is data, not a label.

## The catalog format

One file per locale: `locales/<locale>/stkcraft.tsv`. Three TAB-separated columns:

```text
context	English source text	translation
```

- **English is the key.** There is no English catalog: the source text in the code *is* the English
  interface, and a catalog translates it. That is why an untranslated string shows English rather
  than a placeholder.
- **Contexts** are usually empty. A plain word (`menu`, `preview`) tells apart two meanings of one
  English word. Only `@plural`, `@id` and `@msg` mean something to the code.
- **`{name}` placeholders** must appear in both columns. A translation that drops one is refused:
  a message that cannot be filled in is worse than the English.
- **A trailing `…`** in one column needs one in the other — it means "this opens something", and
  losing it changes what a button promises.
- **`@plural`** rows hold the English `one|other` in the source column and the locale's forms in
  CLDR order, `|`-separated: one form for Chinese or Japanese, two for English-like languages, three
  for Russian, Czech, Polish and Romanian, six for Arabic.
- **`@id language.name`** names the language in its own language, for the picker. Every catalog has
  it; the check refuses one that does not.
- **Escapes** are `\n`, `\t` and `\\`. Nothing else.
- **Partial catalogs are welcome.** A missing row shows English, so a language can land at 5% and be
  useful.

## Commands

```sh
cargo xtask i18n                              # validate every catalog (the gate's step)
cargo xtask i18n report                       # what each language still lacks
cargo xtask i18n translate --language de      # fill what German lacks, with DeepSeek
cargo xtask i18n translate --language zh --dry-run    # show the proposed rows, write nothing
cargo xtask i18n translate --all              # every locale that has a directory
```

`cargo xtask i18n translate` needs `DEEPSEEK_API_KEY`. The key is yours: the project ships one English
source, and translations are produced from it by whoever wants them — a translator, or a model
running on your key. `DEEPSEEK_BASE_URL` and `DEEPSEEK_MODEL` override the endpoint and the model,
exactly as they do for the assistant (`stkcraft ask`).

The translator sends the missing rows in one batch, asks for a JSON object keyed by the English
source, and refuses any row that breaks the format: a dropped placeholder, an invented one, a
missing ellipsis, an empty translation. A refused row is reported by name and left untranslated
rather than written wrong.

## Using a language

The window reads `ui.json` from the platform's configuration directory and offers a language picker
under **Language** in the side panel. The order is: an explicit choice, then the system locale, then
English. A choice is written the moment it is made, because a window that crashes should not forget
the language someone just picked.

The window also reads `locales/<locale>.tsv` from its own configuration directory, so a catalog
generated on this machine works without rebuilding:

```text
Linux    ${XDG_CONFIG_HOME:-~/.config}/stkcraft/locales/de.tsv
macOS    ~/Library/Application Support/ai.storyteller.stkcraft/locales/de.tsv
Windows  %APPDATA%\stkcraft\locales\de.tsv
```

A catalog in the repository is embedded at build time by `apps/stkcraft/build.rs`, which means a
build with no `locales/` directory is a valid English-only build.

## Adding a language

1. Pick the code: lowercase BCP 47, as the family requires — `de`, `zh`, `pt-br`, `zh-hans`,
   `sr-latn`. A script variant and a region are alternatives, not both (`zh-hans`, not `zh-hans-cn`).
2. Make `locales/<code>/stkcraft.tsv` with the header and an `@id	language.name	<name>` row.
3. Run `cargo xtask i18n translate --language <code>` with your key, or fill the rows by hand.
4. Review every row. A machine translation that is wrong is worse than a missing one, because the
   missing one shows English and the wrong one is believed.
5. `cargo xtask i18n` must pass.

## What is not done yet

- **No number or date localization.** A pass time prints as `2024-01-01 09:12:04` in every language.
  The family's shared crate formats numbers per region; this project has not adopted that yet, and
  doing it properly means formatting through one place rather than through `format!("{:.0} km")`.
- **No runtime translation.** A language with no catalog shows English; the window does not ask a
  model for the words itself. The pipeline is a developer task (`cargo xtask i18n translate`), which
  keeps a network call out of the window's startup path.
- **No right-to-left layout.** The catalogs can hold Arabic or Hebrew text; `egui` does not mirror
  the layout, so those translations would render left-to-right and look wrong.
