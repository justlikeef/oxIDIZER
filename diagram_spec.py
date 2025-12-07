diagram_spec = """
@startuml
!theme lightred

title Component Diagram for Web Service

package "Client Application" {
  [Web Browser] -- (HTTP Request) --> [Web Service (ox_webservice)]
}

package "Web Service (ox_webservice)" {
  [Web Service (ox_webservice)] --> [Router]
  [Router] --> [Pipeline Executor]
  [Pipeline Executor] -- (Calls) --> [Modules (Shared Libraries)]
}

package "Modules (Shared Libraries)" {
  [Module Interface] <|-- [ox_webservice_x_forwarded_for]
  [Module Interface] <|-- [ox_content]
  [Module Interface] <|-- [ox_webservice_errorhandler_jinja2]
}

package "Data Flow" {
  [Pipeline State] -- (Contains) --> [Request Data]
  [Pipeline State] -- (Contains) --> [Response Data]
  [Pipeline State] -- (Contains) --> [Module Context]
  [Pipeline State] -- (Owns) --> [Bump Arena]
}

note right of "Pipeline Executor": Manages phases and module execution
note right of "Modules (Shared Libraries)": Implements specific request handling logic
note right of "Bump Arena": Efficient memory allocation for request-scoped data
@enduml
"""