module Page.Dashboard.CreateApp exposing
    ( Model
    , Msg(..)
    , State
    , Step(..)
    , GitHubConnectState
    , init
    , update
    , view
    )

{-| Create app modal for Dashboard page.
-}

import Effect exposing (Effect)
import Html exposing (Html, a, button, div, h2, h3, input, label, option, p, pre, select, span, text)
import Html.Attributes exposing (class, disabled, for, href, id, placeholder, required, selected, target, title, type_, value)
import Html.Events exposing (onClick, onInput, onSubmit)
import Shared


-- MODEL


type alias Model =
    { appName : String
    , step : Step
    , error : Maybe String
    }


type alias State =
    Model


type Step
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


init : Model
init =
    { appName = ""
    , step = EnterName
    , error = Nothing
    }


-- UPDATE


type Msg
    = ShowCreateApp
    | CancelCreateApp
    | AppNameChanged String
    | SubmitAppName
    | StartGitHubConnect
    | GotDeviceFlowStart (Result String Effect.DeviceFlowStartResponse)
    | GotRepoList (Result String (List Effect.RepoInfo))
    | RepoSearchChanged String
    | ChooseRepo Effect.RepoInfo
    | SkipRepoSelection


update : Shared.Model navigationKey -> Msg -> Model -> ( Model, Effect Msg )
update shared msg model =
    case msg of
        ShowCreateApp ->
            ( init
            , Effect.none
            )

        CancelCreateApp ->
            ( model
            , Effect.none
            )

        AppNameChanged name ->
            ( { model | appName = name }
            , Effect.none
            )

        SubmitAppName ->
            if String.isEmpty (String.trim model.appName) then
                ( { model | error = Just "App name is required" }
                , Effect.none
                )

            else
                -- Check GitHub status and proceed accordingly
                case shared.githubStatus of
                    Shared.GitHubConnected _ ->
                        -- Already connected, fetch repos
                        case shared.token of
                            Just token ->
                                ( { model | step = SelectRepo [] "", error = Nothing }
                                , Effect.FetchRepos token GotRepoList
                                )

                            Nothing ->
                                ( model, Effect.none )

                    Shared.GitHubNotConnected ->
                        -- Show GitHub connect option
                        ( { model
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
                        , Effect.none
                        )

                    Shared.GitHubUnknown ->
                        -- Still loading, just go to CheckingGitHub state
                        -- Dashboard will handle fetching the GitHub status
                        ( { model | step = CheckingGitHub, error = Nothing }
                        , Effect.none
                        )

        StartGitHubConnect ->
            case shared.token of
                Just token ->
                    ( model
                    , Effect.StartDeviceFlow token GotDeviceFlowStart
                    )

                Nothing ->
                    ( model, Effect.none )

        GotDeviceFlowStart result ->
            case (shared.token, result) of
                (Just token, Ok response) ->
                    ( { model
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
                    , Effect.StartGitHubPolling
                        token
                        response.deviceCode
                        response.interval
                        response.expiresIn
                    )

                (Just _, Err err) ->
                    ( { model | error = Just err }
                    , Effect.none
                    )

                _ ->
                    ( model, Effect.none )

        GotRepoList result ->
            case result of
                Ok repos ->
                    case model.step of
                        SelectRepo _ query ->
                            ( { model | step = SelectRepo repos query }
                            , Effect.none
                            )

                        _ ->
                            ( { model | step = SelectRepo repos "" }
                            , Effect.none
                            )

                Err err ->
                    ( { model | error = Just err }
                    , Effect.none
                    )

        RepoSearchChanged query ->
            case model.step of
                SelectRepo repos _ ->
                    ( { model | step = SelectRepo repos query }
                    , Effect.none
                    )

                _ ->
                    ( model, Effect.none )

        ChooseRepo repo ->
            case shared.token of
                Just token ->
                    ( { model | step = Creating, error = Nothing }
                    , Effect.none
                    )

                Nothing ->
                    ( model, Effect.none )

        SkipRepoSelection ->
            case shared.token of
                Just token ->
                    ( { model | step = Creating, error = Nothing }
                    , Effect.none
                    )

                Nothing ->
                    ( model, Effect.none )


-- VIEW


view : Shared.Model navigationKey -> Model -> Html Msg
view shared model =
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
            , viewError model.error
            , case model.step of
                EnterName ->
                    viewEnterName model

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


viewEnterName : Model -> Html Msg
viewEnterName model =
    div [ class "py-4" ]
        [ h3 [ class "text-base font-medium text-litehouse-text mb-4" ] [ text "Step 1: Name your app" ]
        , Html.form [ onSubmit SubmitAppName ]
            [ div [ class "mb-4" ]
                [ label [ for "appName", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "App Name" ]
                , input
                    [ type_ "text"
                    , id "appName"
                    , value model.appName
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
