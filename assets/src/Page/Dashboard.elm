module Page.Dashboard exposing
    ( Model
    , Msg(..)
    , init
    , update
    , view
    , save
    , load
    )

import Effect exposing (Effect)
import Html exposing (Html, a, aside, button, div, footer, h1, h2, h3, header, input, label, main_, nav, option, p, pre, select, span, text)
import Html.Attributes exposing (class, disabled, for, href, id, placeholder, required, selected, target, title, type_, value)
import Html.Events exposing (onClick, onInput, onSubmit)
import Json.Decode as Decode
import Shared


-- MODEL


type alias Model =
    { view : DashboardView
    , apps : List Effect.AppInfo
    , appsLoading : Bool
    , activeSidebarItem : SidebarItem
    }


type DashboardView
    = AppsListView
    | CreateAppView CreateAppState
    | AppDetailView AppDetailState


type alias CreateAppState =
    { appName : String
    , step : CreateAppStep
    , error : Maybe String
    }


type CreateAppStep
    = EnterName
    | CheckingGitHub
    | ConnectGitHub GitHubConnectState
    | SelectRepo (List Effect.RepoInfo) String
    | Creating


type alias GitHubConnectState =
    { userCode : String
    , verificationUri : String
    , deviceCode : String
    , expiresIn : Int
    , interval : Int
    , polling : Bool
    }


type alias AppDetailState =
    { app : Effect.AppDetail
    , logs : String
    , logsLoading : Bool
    , logsView : LogsView
    , builds : List Effect.BuildInfo
    , selectedBuildId : Maybe String
    , buildLogs : String
    , buildLogsLoading : Bool
    , actionInProgress : Maybe AppAction
    , error : Maybe String
    }


type AppAction
    = Starting
    | Stopping
    | Building
    | Deleting


type LogsView
    = RuntimeLogs
    | BuildLogs


type SidebarItem
    = MyApps
    | Activity
    | Backups
    | Settings


emptyCreateAppState : CreateAppState
emptyCreateAppState =
    { appName = ""
    , step = EnterName
    , error = Nothing
    }


-- INIT


init : Shared.Model -> ( Model, Effect Msg )
init shared =
    let
        effects =
            case ( shared.token, shared.user ) of
                ( Just token, Just _ ) ->
                    Effect.batch
                        [ Effect.FetchApps token GotApps
                        , Effect.FetchGitHubStatus token GotGitHubStatus
                        ]

                _ ->
                    Effect.none
    in
    ( { view = AppsListView
      , apps = []
      , appsLoading = True
      , activeSidebarItem = MyApps
      }
    , effects
    )


-- UPDATE


type Msg
    = Logout
    | GotApps (Result String (List Effect.AppInfo))
    | GotGitHubStatus (Result String Effect.GitHubStatusResponse)
      -- Create app flow
    | ShowCreateApp
    | CancelCreateApp
    | AppNameChanged String
    | SubmitAppName
    | StartGitHubConnect
    | GotDeviceFlowStart (Result String Effect.DeviceFlowStartResponse)
    | GotGitHubSSEEvent Decode.Value
    | GotRepoList (Result String (List Effect.RepoInfo))
    | RepoSearchChanged String
    | ChooseRepo Effect.RepoInfo
    | SkipRepoSelection
    | GotAppCreated (Result String Effect.AppInfo)
      -- App detail
    | ViewAppDetail String
    | GotAppDetail (Result String Effect.AppDetail)
    | BackToApps
    | RefreshAppDetail
    | StartApp
    | StopApp
    | BuildApp
    | DeleteApp
    | ConfirmDeleteApp
    | CancelDeleteApp
    | GotAppStarted (Result String String)
    | GotAppStopped (Result String String)
    | GotAppBuilt (Result String String)
    | GotAppDeleted (Result String String)
    | FetchLogs
    | GotLogs (Result String String)
      -- Build logs
    | SwitchLogsView LogsView
    | GotBuilds (Result String (List Effect.BuildInfo))
    | SelectBuild String
    | GotBuildLogs (Result String String)


update : Shared.Model -> Msg -> Model -> ( Model, Effect Msg )
update shared msg model =
    case msg of
        Logout ->
            ( model
            , Effect.batch
                [ Effect.ClearToken
                , Effect.PushUrl "/login"
                ]
            )

        GotApps result ->
            case result of
                Ok apps ->
                    ( { model | apps = apps, appsLoading = False }
                    , Effect.none
                    )

                Err _ ->
                    ( { model | appsLoading = False }
                    , Effect.none
                    )

        GotGitHubStatus result ->
            case result of
                Ok response ->
                    let
                        status =
                            if response.connected then
                                case response.username of
                                    Just username ->
                                        Shared.GitHubConnected username

                                    Nothing ->
                                        Shared.GitHubConnected ""

                            else
                                Shared.GitHubNotConnected
                    in
                    ( model
                    , Effect.none
                    )

                Err _ ->
                    ( model
                    , Effect.none
                    )

        ShowCreateApp ->
            ( { model | view = CreateAppView emptyCreateAppState }
            , Effect.none
            )

        CancelCreateApp ->
            ( { model | view = AppsListView }
            , Effect.none
            )

        AppNameChanged name ->
            case model.view of
                CreateAppView createState ->
                    ( { model | view = CreateAppView { createState | appName = name } }
                    , Effect.none
                    )

                _ ->
                    ( model, Effect.none )

        SubmitAppName ->
            case ( model.view, shared.token ) of
                ( CreateAppView createState, Just token ) ->
                    if String.isEmpty (String.trim createState.appName) then
                        ( { model
                            | view = CreateAppView { createState | error = Just "App name is required" }
                          }
                        , Effect.none
                        )

                    else
                        -- Check GitHub status and proceed accordingly
                        case shared.githubStatus of
                            Shared.GitHubConnected _ ->
                                -- Already connected, fetch repos
                                ( { model
                                    | view = CreateAppView { createState | step = SelectRepo [] "", error = Nothing }
                                  }
                                , Effect.FetchRepos token GotRepoList
                                )

                            Shared.GitHubNotConnected ->
                                -- Show GitHub connect option
                                ( { model
                                    | view =
                                        CreateAppView
                                            { createState
                                                | step =
                                                    ConnectGitHub
                                                        { userCode = ""
                                                        , verificationUri = ""
                                                        , deviceCode = ""
                                                        , expiresIn = 0
                                                        , interval = 5
                                                        , polling = False
                                                        }
                                                , error = Nothing
                                            }
                                  }
                                , Effect.none
                                )

                            Shared.GitHubUnknown ->
                                -- Still loading, check again
                                ( { model
                                    | view = CreateAppView { createState | step = CheckingGitHub, error = Nothing }
                                  }
                                , Effect.FetchGitHubStatus token GotGitHubStatus
                                )

                _ ->
                    ( model, Effect.none )

        StartGitHubConnect ->
            case shared.token of
                Just token ->
                    ( model
                    , Effect.StartDeviceFlow token GotDeviceFlowStart
                    )

                Nothing ->
                    ( model, Effect.none )

        GotDeviceFlowStart result ->
            case ( model.view, shared.token ) of
                ( CreateAppView createState, Just token ) ->
                    case result of
                        Ok response ->
                            ( { model
                                | view =
                                    CreateAppView
                                        { createState
                                            | step =
                                                ConnectGitHub
                                                    { userCode = response.userCode
                                                    , verificationUri = response.verificationUri
                                                    , deviceCode = response.deviceCode
                                                    , expiresIn = response.expiresIn
                                                    , interval = response.interval
                                                    , polling = True
                                                    }
                                            , error = Nothing
                                        }
                              }
                            , Effect.StartGitHubSSE
                                { token = token
                                , deviceCode = response.deviceCode
                                , interval = response.interval
                                , expiresIn = response.expiresIn
                                }
                            )

                        Err err ->
                            ( { model
                                | view = CreateAppView { createState | error = Just err }
                              }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )

        GotGitHubSSEEvent value ->
            case ( model.view, shared.token ) of
                ( CreateAppView createState, Just token ) ->
                    case Decode.decodeValue sseEventDecoder value of
                        Ok event ->
                            case event.eventType of
                                "success" ->
                                    -- Parse the data as JSON to get username
                                    case Decode.decodeString (Decode.field "username" Decode.string) event.data of
                                        Ok username ->
                                            ( { model
                                                | view =
                                                    CreateAppView
                                                        { createState
                                                            | step = SelectRepo [] ""
                                                            , error = Nothing
                                                        }
                                              }
                                            , Effect.FetchRepos token GotRepoList
                                            )

                                        Err _ ->
                                            -- Fallback: still a success, just no username parsed
                                            ( { model
                                                | view =
                                                    CreateAppView
                                                        { createState
                                                            | step = SelectRepo [] ""
                                                            , error = Nothing
                                                        }
                                              }
                                            , Effect.FetchRepos token GotRepoList
                                            )

                                "error" ->
                                    -- Stop polling and show error
                                    case createState.step of
                                        ConnectGitHub ghState ->
                                            ( { model
                                                | view =
                                                    CreateAppView
                                                        { createState
                                                            | step = ConnectGitHub { ghState | polling = False }
                                                            , error = Just event.data
                                                        }
                                              }
                                            , Effect.none
                                            )

                                        _ ->
                                            ( model, Effect.none )

                                "pending" ->
                                    -- Still waiting, keep polling (subscription handles this)
                                    ( model, Effect.none )

                                _ ->
                                    -- Unknown event type, ignore
                                    ( model, Effect.none )

                        Err _ ->
                            -- Failed to decode event, ignore
                            ( model, Effect.none )

                _ ->
                    ( model, Effect.none )

        GotRepoList result ->
            case model.view of
                CreateAppView createState ->
                    case result of
                        Ok repos ->
                            case createState.step of
                                SelectRepo _ query ->
                                    ( { model
                                        | view = CreateAppView { createState | step = SelectRepo repos query }
                                      }
                                    , Effect.none
                                    )

                                _ ->
                                    ( { model
                                        | view = CreateAppView { createState | step = SelectRepo repos "" }
                                      }
                                    , Effect.none
                                    )

                        Err err ->
                            ( { model
                                | view = CreateAppView { createState | error = Just err }
                              }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )

        RepoSearchChanged query ->
            case model.view of
                CreateAppView createState ->
                    case createState.step of
                        SelectRepo repos _ ->
                            ( { model
                                | view = CreateAppView { createState | step = SelectRepo repos query }
                              }
                            , Effect.none
                            )

                        _ ->
                            ( model, Effect.none )

                _ ->
                    ( model, Effect.none )

        ChooseRepo repo ->
            case ( model.view, shared.token ) of
                ( CreateAppView createState, Just token ) ->
                    ( { model
                        | view = CreateAppView { createState | step = Creating, error = Nothing }
                      }
                    , Effect.CreateAppWithRepo token createState.appName repo.fullName GotAppCreated
                    )

                _ ->
                    ( model, Effect.none )

        SkipRepoSelection ->
            case ( model.view, shared.token ) of
                ( CreateAppView createState, Just token ) ->
                    ( { model
                        | view = CreateAppView { createState | step = Creating, error = Nothing }
                      }
                    , Effect.CreateApp token createState.appName GotAppCreated
                    )

                _ ->
                    ( model, Effect.none )

        GotAppCreated result ->
            case result of
                Ok app ->
                    -- App created, add to list and go back to apps view
                    ( { model
                        | apps = app :: model.apps
                        , view = AppsListView
                      }
                    , Effect.none
                    )

                Err err ->
                    case model.view of
                        CreateAppView createState ->
                            ( { model
                                | view =
                                    CreateAppView
                                        { createState
                                            | step = EnterName
                                            , error = Just err
                                        }
                              }
                            , Effect.none
                            )

                        _ ->
                            ( model, Effect.none )

        ViewAppDetail appName ->
            case shared.token of
                Just token ->
                    ( model
                    , Effect.batch
                        [ Effect.PushUrl ("/apps/" ++ appName)
                        , Effect.FetchAppDetail token appName GotAppDetail
                        ]
                    )

                Nothing ->
                    ( model, Effect.none )

        GotAppDetail result ->
            case result of
                Ok app ->
                    ( { model
                        | view =
                            AppDetailView
                                { app = app
                                , logs = ""
                                , logsLoading = False
                                , logsView = RuntimeLogs
                                , builds = []
                                , selectedBuildId = Nothing
                                , buildLogs = ""
                                , buildLogsLoading = False
                                , actionInProgress = Nothing
                                , error = Nothing
                                }
                      }
                    , Effect.none
                    )

                Err err ->
                    -- Failed to fetch app detail, stay on current view
                    ( model, Effect.none )

        BackToApps ->
            ( { model | view = AppsListView }
            , Effect.PushUrl "/dashboard"
            )

        RefreshAppDetail ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( model
                    , Effect.FetchAppDetail token detailState.app.name GotAppDetail
                    )

                _ ->
                    ( model, Effect.none )

        StartApp ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view =
                            AppDetailView
                                { detailState | actionInProgress = Just Starting, error = Nothing }
                      }
                    , Effect.StartApp token detailState.app.name GotAppStarted
                    )

                _ ->
                    ( model, Effect.none )

        StopApp ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view =
                            AppDetailView
                                { detailState | actionInProgress = Just Stopping, error = Nothing }
                      }
                    , Effect.StopApp token detailState.app.name GotAppStopped
                    )

                _ ->
                    ( model, Effect.none )

        BuildApp ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view =
                            AppDetailView
                                { detailState | actionInProgress = Just Building, error = Nothing }
                      }
                    , Effect.BuildApp token detailState.app.name GotAppBuilt
                    )

                _ ->
                    ( model, Effect.none )

        ConfirmDeleteApp ->
            -- In a real implementation, we'd show a confirmation dialog
            -- For now, just proceed with deletion
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view =
                            AppDetailView
                                { detailState | actionInProgress = Just Deleting, error = Nothing }
                      }
                    , Effect.DeleteApp token detailState.app.name GotAppDeleted
                    )

                _ ->
                    ( model, Effect.none )

        CancelDeleteApp ->
            -- Not implemented yet
            ( model, Effect.none )

        DeleteApp ->
            -- Not implemented yet
            ( model, Effect.none )

        GotAppStarted result ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    case result of
                        Ok _ ->
                            ( { model
                                | view = AppDetailView { detailState | actionInProgress = Nothing }
                              }
                            , Effect.FetchAppDetail token detailState.app.name GotAppDetail
                            )

                        Err err ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | actionInProgress = Nothing
                                            , error = Just err
                                        }
                              }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )

        GotAppStopped result ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    case result of
                        Ok _ ->
                            ( { model
                                | view = AppDetailView { detailState | actionInProgress = Nothing }
                              }
                            , Effect.FetchAppDetail token detailState.app.name GotAppDetail
                            )

                        Err err ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | actionInProgress = Nothing
                                            , error = Just err
                                        }
                              }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )

        GotAppBuilt result ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    case result of
                        Ok _ ->
                            ( { model
                                | view = AppDetailView { detailState | actionInProgress = Nothing }
                              }
                            , Effect.batch
                                [ Effect.FetchAppDetail token detailState.app.name GotAppDetail
                                , Effect.FetchBuilds token detailState.app.name GotBuilds
                                ]
                            )

                        Err err ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | actionInProgress = Nothing
                                            , error = Just err
                                        }
                              }
                            , Effect.FetchBuilds token detailState.app.name GotBuilds
                            )

                _ ->
                    ( model, Effect.none )

        GotAppDeleted result ->
            case result of
                Ok _ ->
                    -- App deleted, go back to apps list
                    ( { model | view = AppsListView }
                    , Effect.PushUrl "/dashboard"
                    )

                Err err ->
                    case model.view of
                        AppDetailView detailState ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | actionInProgress = Nothing
                                            , error = Just err
                                        }
                              }
                            , Effect.none
                            )

                        _ ->
                            ( model, Effect.none )

        FetchLogs ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view = AppDetailView { detailState | logsLoading = True }
                      }
                    , Effect.FetchLogs token detailState.app.name GotLogs
                    )

                _ ->
                    ( model, Effect.none )

        GotLogs result ->
            case model.view of
                AppDetailView detailState ->
                    case result of
                        Ok logs ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | logs = logs
                                            , logsLoading = False
                                        }
                              }
                            , Effect.none
                            )

                        Err _ ->
                            ( { model
                                | view = AppDetailView { detailState | logsLoading = False }
                              }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )

        SwitchLogsView logsView ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    let
                        updatedDetailState =
                            { detailState | logsView = logsView }

                        effect =
                            case logsView of
                                BuildLogs ->
                                    if List.isEmpty detailState.builds then
                                        Effect.FetchBuilds token detailState.app.name GotBuilds

                                    else
                                        Effect.none

                                RuntimeLogs ->
                                    if String.isEmpty detailState.logs then
                                        Effect.FetchLogs token detailState.app.name GotLogs

                                    else
                                        Effect.none
                    in
                    ( { model | view = AppDetailView updatedDetailState }
                    , effect
                    )

                _ ->
                    ( model, Effect.none )

        GotBuilds result ->
            case model.view of
                AppDetailView detailState ->
                    case result of
                        Ok builds ->
                            let
                                selectedBuildId =
                                    case builds of
                                        first :: _ ->
                                            Just first.id

                                        [] ->
                                            Nothing

                                effect =
                                    case ( shared.token, selectedBuildId ) of
                                        ( Just token, Just buildId ) ->
                                            Effect.FetchBuildLogs token detailState.app.name buildId GotBuildLogs

                                        _ ->
                                            Effect.none
                            in
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | builds = builds
                                            , selectedBuildId = selectedBuildId
                                            , buildLogsLoading = selectedBuildId /= Nothing
                                        }
                              }
                            , effect
                            )

                        Err _ ->
                            ( model, Effect.none )

                _ ->
                    ( model, Effect.none )

        SelectBuild buildId ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view =
                            AppDetailView
                                { detailState
                                    | selectedBuildId = Just buildId
                                    , buildLogsLoading = True
                                }
                      }
                    , Effect.FetchBuildLogs token detailState.app.name buildId GotBuildLogs
                    )

                _ ->
                    ( model, Effect.none )

        GotBuildLogs result ->
            case model.view of
                AppDetailView detailState ->
                    case result of
                        Ok logs ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | buildLogs = logs
                                            , buildLogsLoading = False
                                        }
                              }
                            , Effect.none
                            )

                        Err _ ->
                            ( { model
                                | view = AppDetailView { detailState | buildLogsLoading = False }
                              }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )


-- Note: GitHub SSE subscription is handled by Main.elm
-- Main subscribes to the gitHubSSEEvent port and forwards events
-- to the Dashboard's GotGitHubSSEEvent message when appropriate


-- DECODERS


type alias SSEEvent =
    { eventType : String
    , data : String
    }


sseEventDecoder : Decode.Decoder SSEEvent
sseEventDecoder =
    Decode.map2 SSEEvent
        (Decode.field "type" Decode.string)
        (Decode.field "data" Decode.string)


-- VIEW


view : Shared.Model -> Model -> Html Msg
view shared model =
    div [ class "min-h-screen bg-litehouse-bg flex flex-col" ]
        [ viewHeader shared model
        , div [ class "flex flex-1" ]
            [ viewSidebar model
            , main_ [ class "flex-1 p-6" ]
                [ div [ class "max-w-6xl mx-auto" ]
                    [ case model.view of
                        AppsListView ->
                            viewAppsList model

                        CreateAppView createState ->
                            viewCreateApp shared model createState

                        AppDetailView detailState ->
                            viewAppDetail detailState
                    ]
                ]
            ]
        , footer [ class "p-4 text-center border-t border-litehouse-border" ]
            [ viewVersion shared.serverVersion
            ]
        ]


viewHeader : Shared.Model -> Model -> Html Msg
viewHeader shared model =
    let
        userName =
            case shared.user of
                Just user ->
                    user.fullName

                Nothing ->
                    ""
    in
    header [ class "bg-litehouse-surface border-b border-litehouse-border px-6 py-4 flex justify-between items-center" ]
        [ div [ class "flex items-center gap-4" ]
            [ a [ href "/dashboard", class "text-xl font-semibold text-litehouse-text hover:opacity-80 transition-opacity" ] [ text "Litehouse" ]
            ]
        , div [ class "flex items-center gap-4" ]
            [ viewGitHubStatusBadge shared.githubStatus
            , span [ class "text-sm text-litehouse-muted" ] [ text userName ]
            , button
                [ class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                , onClick ShowCreateApp
                ]
                [ text "+ New App" ]
            , button
                [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                , onClick Logout
                ]
                [ text "Logout" ]
            ]
        ]


viewSidebar : Model -> Html Msg
viewSidebar model =
    aside [ class "w-56 bg-litehouse-surface border-r border-litehouse-border p-4" ]
        [ nav [ class "space-y-1" ]
            [ viewSidebarItem "My Apps" MyApps model.activeSidebarItem
            , viewSidebarItem "Activity" Activity model.activeSidebarItem
            , viewSidebarItem "Backups" Backups model.activeSidebarItem
            , viewSidebarItem "Settings" Settings model.activeSidebarItem
            ]
        ]


viewSidebarItem : String -> SidebarItem -> SidebarItem -> Html Msg
viewSidebarItem label item activeItem =
    let
        isActive =
            item == activeItem

        baseClasses =
            "block w-full px-3 py-2 rounded-xl text-sm font-medium transition-colors text-left"

        activeClasses =
            if isActive then
                "bg-litehouse-amber/10 text-litehouse-amber"

            else
                "text-litehouse-muted hover:bg-litehouse-bg hover:text-litehouse-text"
    in
    button [ class (baseClasses ++ " " ++ activeClasses) ] [ text label ]


viewGitHubStatusBadge : Shared.GitHubStatus -> Html Msg
viewGitHubStatusBadge status =
    case status of
        Shared.GitHubConnected username ->
            span [ class "px-2.5 py-1 rounded-full text-xs font-medium bg-litehouse-success/20 text-litehouse-success" ]
                [ text ("GitHub: " ++ username) ]

        Shared.GitHubNotConnected ->
            span [ class "px-2.5 py-1 rounded-full text-xs font-medium bg-litehouse-warning/20 text-litehouse-warning" ]
                [ text "GitHub: Not connected" ]

        Shared.GitHubUnknown ->
            text ""


viewAppsList : Model -> Html Msg
viewAppsList model =
    div []
        [ div [ class "flex justify-between items-center mb-6" ]
            [ h2 [ class "text-xl font-semibold text-litehouse-text" ] [ text "My Apps" ]
            ]
        , if model.appsLoading then
            div [ class "flex flex-col items-center justify-center py-16 text-litehouse-muted" ]
                [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mb-4" ] []
                , p [] [ text "Loading apps..." ]
                ]

          else if List.isEmpty model.apps then
            div [ class "bg-litehouse-surface rounded-2xl shadow-soft p-12 text-center" ]
                [ p [ class "text-litehouse-muted mb-4" ] [ text "No apps yet. Create your first app to get started." ]
                , button
                    [ class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                    , onClick ShowCreateApp
                    ]
                    [ text "Create App" ]
                ]

          else
            div [ class "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4" ]
                (List.map viewAppCard model.apps)
        ]


viewAppCard : Effect.AppInfo -> Html Msg
viewAppCard app =
    a
        [ class "block bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-5 hover:border-litehouse-amber transition-colors cursor-pointer"
        , href ("/apps/" ++ app.name)
        , onClick (ViewAppDetail app.name)
        ]
        [ div [ class "flex justify-between items-start mb-3" ]
            [ h3 [ class "text-base font-semibold text-litehouse-text" ] [ text app.name ]
            , viewStatusBadge app.state
            ]
        , div [ class "text-xs text-litehouse-muted font-mono" ]
            [ text ("ID: " ++ app.id) ]
        ]


viewStatusBadge : String -> Html Msg
viewStatusBadge state =
    let
        ( bgColor, textColor ) =
            case state of
                "running" ->
                    ( "bg-litehouse-success/20", "text-litehouse-success" )

                "starting" ->
                    ( "bg-litehouse-warning/20", "text-litehouse-warning" )

                "building" ->
                    ( "bg-litehouse-warning/20", "text-litehouse-warning" )

                "stopped" ->
                    ( "bg-litehouse-error/20", "text-litehouse-error" )

                "error" ->
                    ( "bg-litehouse-error/20", "text-litehouse-error" )

                _ ->
                    ( "bg-litehouse-border/50", "text-litehouse-muted" )
    in
    span [ class ("px-2.5 py-1 rounded-full text-xs font-medium uppercase " ++ bgColor ++ " " ++ textColor) ]
        [ text state ]


viewAppDetail : AppDetailState -> Html Msg
viewAppDetail state =
    let
        app =
            state.app

        isRunning =
            app.state == "running" || app.state == "starting"

        hasRemote =
            state.app.remote /= Nothing

        actionDisabled =
            state.actionInProgress /= Nothing
    in
    div [ class "space-y-6" ]
        [ -- Header with back button
          div [ class "flex items-center justify-between" ]
            [ div [ class "flex items-center gap-4" ]
                [ button
                    [ class "text-litehouse-slateBlue hover:bg-litehouse-mistBlue px-3 py-1.5 rounded-xl transition-colors text-sm"
                    , onClick BackToApps
                    ]
                    [ text "< Apps" ]
                , h2 [ class "text-2xl font-semibold text-litehouse-text" ] [ text app.name ]
                , viewStatusBadge app.state
                ]
            , button
                [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors disabled:opacity-50"
                , onClick RefreshAppDetail
                , disabled actionDisabled
                ]
                [ text "Refresh" ]
            ]

        -- Error message
        , viewError state.error

        -- Action in progress indicator
        , case state.actionInProgress of
            Just action ->
                div [ class "flex items-center gap-3 p-4 bg-litehouse-warning/10 text-litehouse-warning rounded-xl" ]
                    [ div [ class "w-5 h-5 border-2 border-litehouse-warning/30 border-t-litehouse-warning rounded-full animate-spin-slow" ] []
                    , span []
                        [ text
                            (case action of
                                Starting ->
                                    "Starting app..."

                                Stopping ->
                                    "Stopping app..."

                                Building ->
                                    "Building app (this may take a while)..."

                                Deleting ->
                                    "Deleting app..."
                            )
                        ]
                    ]

            Nothing ->
                text ""

        -- Info section
        , div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ h3 [ class "text-base font-semibold text-litehouse-text mb-4" ] [ text "Information" ]
            , div [ class "grid grid-cols-2 md:grid-cols-4 gap-4" ]
                [ div []
                    [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "ID" ]
                    , span [ class "text-sm text-litehouse-text font-mono break-all" ] [ text app.id ]
                    ]
                , div []
                    [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "State" ]
                    , viewStatusBadge app.state
                    ]
                , div []
                    [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "Port" ]
                    , span [ class "text-sm text-litehouse-text" ]
                        [ text
                            (case app.port_ of
                                Just p ->
                                    String.fromInt p

                                Nothing ->
                                    "Not assigned"
                            )
                        ]
                    ]
                , div []
                    [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "Created" ]
                    , span [ class "text-sm text-litehouse-text" ] [ text app.createdAt ]
                    ]
                ]
            ]

        -- Repository section
        , div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ h3 [ class "text-base font-semibold text-litehouse-text mb-4" ] [ text "Repository" ]
            , case app.remote of
                Just remote ->
                    div [ class "grid grid-cols-2 gap-4" ]
                        [ div []
                            [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "URL" ]
                            , span [ class "text-sm text-litehouse-text font-mono break-all" ] [ text remote.url ]
                            ]
                        , div []
                            [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "Branch" ]
                            , span [ class "text-sm text-litehouse-text" ] [ text remote.branch ]
                            ]
                        ]

                Nothing ->
                    p [ class "text-sm text-litehouse-muted italic" ] [ text "No repository connected" ]
            ]

        -- Actions section
        , div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ h3 [ class "text-base font-semibold text-litehouse-text mb-4" ] [ text "Actions" ]
            , div [ class "flex flex-wrap gap-3" ]
                [ -- Start/Stop button
                  if isRunning then
                    button
                        [ class "px-5 py-2.5 bg-litehouse-warning hover:bg-litehouse-warning/80 text-white font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                        , onClick StopApp
                        , disabled actionDisabled
                        ]
                        [ text "Stop" ]

                  else
                    button
                        [ class "px-5 py-2.5 bg-litehouse-success hover:bg-litehouse-success/80 text-white font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                        , onClick StartApp
                        , disabled actionDisabled
                        ]
                        [ text "Start" ]

                -- Build button (only if has remote)
                , if hasRemote then
                    button
                        [ class "px-5 py-2.5 bg-litehouse-slateBlue hover:bg-litehouse-slateBlue/80 text-white font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                        , onClick BuildApp
                        , disabled actionDisabled
                        ]
                        [ text "Build" ]

                  else
                    text ""

                -- Delete button
                , button
                    [ class "px-5 py-2.5 bg-litehouse-error hover:bg-litehouse-error/80 text-white font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                    , onClick ConfirmDeleteApp
                    , disabled actionDisabled
                    ]
                    [ text "Delete" ]
                ]
            ]

        -- Logs section
        , div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ div [ class "flex justify-between items-center mb-4" ]
                [ h3 [ class "text-base font-semibold text-litehouse-text" ] [ text "Logs" ]
                , div [ class "flex items-center gap-2" ]
                    [ -- Tab buttons
                      div [ class "flex rounded-lg border border-litehouse-border overflow-hidden" ]
                        [ button
                            [ class
                                (if state.logsView == BuildLogs then
                                    "px-3 py-1.5 text-sm bg-litehouse-amber text-white"

                                 else
                                    "px-3 py-1.5 text-sm text-litehouse-muted hover:bg-litehouse-bg"
                                )
                            , onClick (SwitchLogsView BuildLogs)
                            ]
                            [ text "Build" ]
                        , button
                            [ class
                                (if state.logsView == RuntimeLogs then
                                    "px-3 py-1.5 text-sm bg-litehouse-amber text-white"

                                 else
                                    "px-3 py-1.5 text-sm text-litehouse-muted hover:bg-litehouse-bg"
                                )
                            , onClick (SwitchLogsView RuntimeLogs)
                            ]
                            [ text "App" ]
                        ]

                    -- Refresh button (only for runtime logs)
                    , if state.logsView == RuntimeLogs then
                        button
                            [ class "px-3 py-1.5 text-sm border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors disabled:opacity-50"
                            , onClick FetchLogs
                            , disabled state.logsLoading
                            ]
                            [ text "Refresh" ]

                      else
                        text ""
                    ]
                ]

            -- Build selector (only shown when viewing build logs)
            , if state.logsView == BuildLogs && not (List.isEmpty state.builds) then
                div [ class "mb-4" ]
                    [ select
                        [ class "w-full px-3 py-2 bg-litehouse-bg border border-litehouse-border rounded-xl text-litehouse-text text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber"
                        , onInput SelectBuild
                        ]
                        (List.map
                            (\build ->
                                option
                                    [ value build.id
                                    , selected (state.selectedBuildId == Just build.id)
                                    ]
                                    [ text (String.left 8 (Maybe.withDefault "unknown" build.gitCommit) ++ " - " ++ Maybe.withDefault "no tag" build.imageTag ++ " (" ++ formatBuildDate build.createdAt ++ ")") ]
                            )
                            state.builds
                        )
                    ]

              else
                text ""

            -- Log content
            , viewLogsContent state
            ]
        ]


viewLogsContent : AppDetailState -> Html Msg
viewLogsContent state =
    case state.logsView of
        RuntimeLogs ->
            if state.logsLoading then
                div [ class "flex items-center justify-center gap-3 py-10 text-litehouse-muted" ]
                    [ div [ class "w-5 h-5 border-2 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow" ] []
                    , span [] [ text "Loading logs..." ]
                    ]

            else if String.isEmpty state.logs then
                div [ class "py-10 text-center text-litehouse-muted" ]
                    [ text "No runtime logs available" ]

            else
                pre [ class "bg-gray-900 text-gray-300 font-mono text-xs p-4 rounded-xl overflow-auto max-h-96 whitespace-pre-wrap break-all" ]
                    [ text state.logs ]

        BuildLogs ->
            if state.actionInProgress == Just Building then
                div [ class "flex items-center justify-center gap-3 py-10 text-litehouse-muted" ]
                    [ div [ class "w-5 h-5 border-2 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow" ] []
                    , span [] [ text "Building..." ]
                    ]

            else if state.buildLogsLoading then
                div [ class "flex items-center justify-center gap-3 py-10 text-litehouse-muted" ]
                    [ div [ class "w-5 h-5 border-2 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow" ] []
                    , span [] [ text "Loading build logs..." ]
                    ]

            else if List.isEmpty state.builds then
                div [ class "py-10 text-center text-litehouse-muted" ]
                    [ text "No builds yet. Click Build to create your first build." ]

            else if String.isEmpty state.buildLogs then
                div [ class "py-10 text-center text-litehouse-muted" ]
                    [ text "No logs available for this build" ]

            else
                pre [ class "bg-gray-900 text-gray-300 font-mono text-xs p-4 rounded-xl overflow-auto max-h-96 whitespace-pre-wrap break-all" ]
                    [ text state.buildLogs ]


formatBuildDate : String -> String
formatBuildDate isoDate =
    -- Simple formatting: just take the date part (first 10 characters)
    String.left 10 isoDate


viewCreateApp : Shared.Model -> Model -> CreateAppState -> Html Msg
viewCreateApp shared model createState =
    div [ class "max-w-xl" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ div [ class "flex justify-between items-center mb-6 pb-4 border-b border-litehouse-border" ]
                [ h2 [ class "text-xl font-semibold text-litehouse-text" ] [ text "Create New App" ]
                , button
                    [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                    , onClick CancelCreateApp
                    ]
                    [ text "Cancel" ]
                ]
            , viewError createState.error
            , case createState.step of
                EnterName ->
                    viewEnterName createState

                CheckingGitHub ->
                    viewCheckingGitHub

                ConnectGitHub ghState ->
                    viewConnectGitHub ghState

                SelectRepo repos query ->
                    viewSelectRepo repos query

                Creating ->
                    viewCreating
            ]
        ]


viewEnterName : CreateAppState -> Html Msg
viewEnterName createState =
    div [ class "py-4" ]
        [ h3 [ class "text-base font-medium text-litehouse-text mb-4" ] [ text "Step 1: Name your app" ]
        , Html.form [ onSubmit SubmitAppName ]
            [ div [ class "mb-4" ]
                [ label [ for "appName", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "App Name" ]
                , input
                    [ type_ "text"
                    , id "appName"
                    , value createState.appName
                    , onInput AppNameChanged
                    , placeholder "my-awesome-app"
                    , required True
                    , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50"
                    ]
                    []
                , p [ class "mt-1 text-xs text-litehouse-muted" ] [ text "Use lowercase letters, numbers, and hyphens only" ]
                ]
            , button
                [ type_ "submit"
                , class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                ]
                [ text "Next" ]
            ]
        ]


viewCheckingGitHub : Html Msg
viewCheckingGitHub =
    div [ class "py-8 text-center" ]
        [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mx-auto mb-4" ] []
        , p [ class "text-litehouse-muted" ] [ text "Checking GitHub connection..." ]
        ]


viewConnectGitHub : GitHubConnectState -> Html Msg
viewConnectGitHub ghState =
    div [ class "py-4" ]
        [ h3 [ class "text-base font-medium text-litehouse-text mb-4" ] [ text "Step 2: Connect GitHub (Optional)" ]
        , if String.isEmpty ghState.userCode then
            div [ class "text-center py-4" ]
                [ p [ class "text-litehouse-muted mb-4" ] [ text "Connect your GitHub account to import repositories." ]
                , div [ class "flex justify-center gap-3" ]
                    [ button
                        [ class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                        , onClick StartGitHubConnect
                        ]
                        [ text "Connect GitHub" ]
                    , button
                        [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                        , onClick SkipRepoSelection
                        ]
                        [ text "Skip" ]
                    ]
                ]

          else
            div [ class "text-center py-4" ]
                [ p [ class "text-litehouse-muted mb-2" ] [ text "Go to GitHub and enter this code:" ]
                , div [ class "text-3xl font-bold font-mono tracking-widest py-5 px-6 bg-litehouse-bg rounded-xl my-4 text-litehouse-text" ]
                    [ text ghState.userCode ]
                , p [ class "text-litehouse-muted" ]
                    [ text "Open "
                    , a [ href ghState.verificationUri, target "_blank", class "text-litehouse-slateBlue hover:underline" ] [ text ghState.verificationUri ]
                    , text " and enter the code above."
                    ]
                , if ghState.polling then
                    div [ class "flex items-center justify-center gap-3 mt-4 text-litehouse-muted" ]
                        [ div [ class "w-5 h-5 border-2 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow" ] []
                        , span [] [ text "Waiting for authorization..." ]
                        ]

                  else
                    button
                        [ class "mt-4 px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                        , onClick SkipRepoSelection
                        ]
                        [ text "Skip" ]
                ]
        ]


viewSelectRepo : List Effect.RepoInfo -> String -> Html Msg
viewSelectRepo repos query =
    let
        filteredRepos =
            if String.isEmpty query then
                repos

            else
                List.filter
                    (\repo ->
                        String.contains (String.toLower query) (String.toLower repo.name)
                            || String.contains (String.toLower query) (String.toLower repo.fullName)
                    )
                    repos
    in
    div [ class "py-4" ]
        [ h3 [ class "text-base font-medium text-litehouse-text mb-4" ] [ text "Step 3: Select Repository (Optional)" ]
        , div [ class "mb-4" ]
            [ input
                [ type_ "text"
                , placeholder "Search repositories..."
                , value query
                , onInput RepoSearchChanged
                , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50"
                ]
                []
            ]
        , if List.isEmpty repos then
            div [ class "py-10 text-center" ]
                [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mx-auto mb-4" ] []
                , p [ class "text-litehouse-muted" ] [ text "Loading repositories..." ]
                ]

          else
            div []
                [ div [ class "max-h-80 overflow-y-auto border border-litehouse-border rounded-xl" ]
                    (List.map viewRepoItem filteredRepos)
                , div [ class "mt-4 text-center" ]
                    [ button
                        [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                        , onClick SkipRepoSelection
                        ]
                        [ text "Create without repository" ]
                    ]
                ]
        ]


viewRepoItem : Effect.RepoInfo -> Html Msg
viewRepoItem repo =
    div
        [ class "px-4 py-3 border-b border-litehouse-border last:border-b-0 cursor-pointer hover:bg-litehouse-bg transition-colors"
        , onClick (ChooseRepo repo)
        ]
        [ div [ class "flex justify-between items-center mb-1" ]
            [ span [ class "font-medium text-litehouse-text" ] [ text repo.fullName ]
            , if repo.private then
                span [ class "px-2 py-0.5 rounded text-xs font-medium bg-litehouse-warning/20 text-litehouse-warning" ] [ text "Private" ]

              else
                span [ class "px-2 py-0.5 rounded text-xs font-medium bg-litehouse-border/50 text-litehouse-muted" ] [ text "Public" ]
            ]
        , case repo.description of
            Just desc ->
                p [ class "text-sm text-litehouse-muted truncate mb-1" ] [ text desc ]

            Nothing ->
                text ""
        , span [ class "text-xs text-litehouse-muted" ] [ text ("Default branch: " ++ repo.defaultBranch) ]
        ]


viewCreating : Html Msg
viewCreating =
    div [ class "py-8 text-center" ]
        [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mx-auto mb-4" ] []
        , p [ class "text-litehouse-muted" ] [ text "Creating app..." ]
        ]


viewError : Maybe String -> Html Msg
viewError maybeError =
    case maybeError of
        Just error ->
            div [ class "bg-litehouse-error/10 text-litehouse-error p-3 rounded-xl mb-4 text-sm text-left" ] [ text error ]

        Nothing ->
            text ""


viewVersion : String -> Html Msg
viewVersion version =
    if String.isEmpty version then
        text ""

    else
        p [ class "text-xs text-litehouse-muted" ] [ text ("v" ++ version) ]


-- PAGE LIFECYCLE


{-| Save page state to global state when leaving the page
-}
save : Model -> Shared.Model -> Shared.Model
save model shared =
    -- GitHub status is already saved in Shared via update messages
    shared


{-| Load data from global state when entering the page
-}
load : Shared.Model -> Model -> Model
load shared model =
    -- Nothing specific to load, token and user come from Shared
    model
