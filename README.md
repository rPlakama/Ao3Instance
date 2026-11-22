this project is designed to run as a webapplication that can be acessed thought login;, providing a frontend for AO3 with additional capabilities, such as local caching for fanfics (in server) and saving scroll position of they, and other capabilities, even standing after the AO3 is down, providing a cool and elegant fanfic consumation.

Features:

 - Accounts
 - Caching
 - Local storage of fanfics that user acess
 - Full IMPL of Ao3
 - Search <--> AO3 (server)
 - Full Reader, EPUB (Main) <-- as caching
 - In Rust
 - SQLX as DB;


 Arch ref1; https://github.com/otwcode/otwarchive
 Arch ref2; https://github.com/Kareadita/Kavita <-- ePUB reading desing & features;

 This project shall utilizate flake.nix && Kunobi Kache (https://ninja.kunobi.com/kache) for Local builds. The project shall also use strict Clippy rules.
