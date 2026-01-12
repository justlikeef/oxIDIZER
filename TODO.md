* [x] ox_webserver does not return the error message if no error_handler is loaded. There should be a default error generator if nothing runs during errorhandling
* [x] THIS SHOULD NOT BE DONE. NEED URL FOR OTHER THINGS LIKE AUTHORIZATION. ~~Only run uri regex for pre_content, content, and post_content~~
* [x] Allow for url based configuration
* [ ] Modules:
  * [x] take the IP in the X-Forwarded-For header and puts it in the source IP field, and a module that then puts it back.
  * [x] simple ip based authortization
    * [x] Create module
    * [x] Symlink config to active directory
  * [ ] user authentication via
    * [ ] file
    * [ ] DB
    * [ ] LDAP/sLDAP
  * [ ] user authorization via config, groups, group mapping via above
* [x] Allow full path to module .so
* [x] Refactor ox_content into seperate modules
  * [x] populat ox_stream mimetypes
* [x] create config processing library that all modules can use
  * [x] should be able to include other config files
  * [x] should be able to do replacements, for instance, the ability to set the root directory for the server config, etc.
* [x] add websockets support
* [x] The content modules should be able to respect that there is existing content that has already been rendered.* [x] Implement Data Streaming (OxStream) - see [streaming_data.md](docs/design_proposals/streaming_data.md)
* [x] Change the pipeline is_modified to a list of flags that can be quickly added to, searched, and removed from. Format of the flags is most dependent on speed. Is modified should become a flag called "content_modified" that is checked. Errorhandlers should set a flag called "error_handled" that is checked by other error handlers.
* [x] Revert changes to ox_package_manager returning 200 for errors. JS should correctly handle http errors.
* [x] Write an error handler for header: Accept: application/json and ?format=json to return json instead of html. This should run before ox_webservice_errorhandler_jinja2 or ox_webservice_errorhandler_stream. The json error handler should not detect the header or request, that should be handled through routing. There should be a config option to append or replace successful http status to the existing body and to replace or append error http status to the existing body.
* [x] How should errors be handled for websockets? Should error when the proto is websockets be handled by the json error handler?
