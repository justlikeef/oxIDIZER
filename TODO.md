* [x] ox_webserver does not return the error message if no error_handler is loaded.  There should be a default error generator if nothing runs during errorhandling
* [x] THIS SHOULD NOT BE DONE.  NEED URL FOR OTHER THINGS LIKE AUTHORIZATION. ~~Only run uri regex for pre_content, content, and post_content~~
* [x] Allow for url based configuration
* [ ] Modules:
  * [x] take the IP in the X-Forwarded-For header and puts it in the source IP field, and a module that then puts it back.
  * [ ] simple ip based authortization
  * [ ] user authentication via file, DB, LDAP/sLDAP
  * [ ] user authorization via config, groups, group mapping via above
* [ ] Allow full path to module .so
* [x] Refactor ox_content into seperate modules
  * [x] populat ox_stream mimetypes
* [x] create config processing library that all modules can use
  * [x] should be able to include other config files
  * [x] should be able to do replacements, for instance, the ability to set the root directory for the server config, etc.
* [x] add websockets support
* [ ] The content modules should be able to respect that there is existing content that has already been rendered.