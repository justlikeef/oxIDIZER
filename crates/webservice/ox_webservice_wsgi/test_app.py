def application(environ, start_response):
    """
    Simple WSGI Application for ox_wsgi_module verification.
    """
    status = '200 OK'
    response_headers = [('Content-type', 'text/plain')]
    start_response(status, response_headers)
    
    # Return a simple body
    return [b"Hello, WSGI World!\n", b"Served by oxIDIZER.\n"]
