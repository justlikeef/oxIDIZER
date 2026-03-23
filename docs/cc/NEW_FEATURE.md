New feature:
1) manifest parser
    define structure of manifest
{
    commandset : {
        [commandplugin1] : {
            [param1] : [value1],
            [param2] : [value2]
        },
        [commandplugin2] : {
            [param1] : [value1],
            [param2] : [param2]
        },
    }
}
plugins called with same name a command and pass parameters.
Need some way to communicate state between plugins as needed.                  

2) Session authorization
Command server opens a session with the broker.  Session is approved through interface like a template manifest would be.
Manifests submitted using that session are autoapproved until the session is closed.