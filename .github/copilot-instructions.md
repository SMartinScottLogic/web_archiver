* Follow rust best practices throughout.
* Only minimal code should be in main.rs. The majority of functional code should be in lib.rs or associated modules.
* Unit tests should come at the end of the file.
* After changes, ask the user whether tests should be run.
* For coverage, run 'cargo tarpaulin --out Xml'