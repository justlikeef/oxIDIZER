* [x] ox_webserver does not return the error message if no error_handler is loaded.  There should be a default error generator if nothing runs during errorhandling
* [x] THIS SHOULD NOT BE DONE.  NEED URL FOR OTHER THINGS LIKE AUTHORIZATION. ~~Only run uri regex for pre_content, content, and post_content~~
* [x] Allow for url based configuration
* [ ] Modules:
  * [ ] take the IP in the X-Forwarded-For header and puts it in the source IP field, and a module that then puts it back.
  * [ ] simple ip based authortization
  * [ ] user authentication via file, DB, LDAP/sLDAP
  * [ ] user authorization via config, groups, group mapping via above
* [ ] Allow full path to module .so
* [ ] Refactor ox_content into seperate modules
  * [ ] populat ox_stream mimetypes
* [ ] create config processing library that all modules can use
  * [ ] should be able to include other config files
  * [ ] should be able to do replacements, for instance, the ability to set the root directory for the server config, etc.
* [ ] add websockets support