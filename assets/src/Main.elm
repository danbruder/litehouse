module Main exposing (main, init, initForTesting, update, view, Model, Msg(..), Flags)

import Browser
import Browser.Navigation as Nav
import Effect
import Effect exposing (ServerStatus, TokenVerificationResponse, TokenPair)
import Effects
import Html exposing (Html, div, p, text)
import Html.Attributes exposing (class)
import Http
import Json.Decode as Decode
import Json.Encode as Encode
import Page.Dashboard
import Page.Login
import Page.Settings
import Page.Setup
import Ports
import Route
import Shared

import Time

import Url



-- MAIN


main : Program Flags (Model Nav.Key) Msg
main =
    let
        performEffectForMain navKey effect =
            Effects.performEffect navKey effect
    in
    Browser.application
        { init = \flags url key -> init flags url key |> Tuple.mapSecond (performEffectForMain key)
        , update = \msg model -> update msg model |> Tuple.mapSecond (performEffectForMain model.shared.navKey)
        , view = view
        , subscriptions = subscriptions
        , onUrlChange = UrlChanged
        , onUrlRequest = LinkClicked
        }



-- FLAGS


type alias Flags =
    { token : Maybe String
    }



-- MODEL


type alias Model navigationKey =
    { shared : Shared.Model navigationKey
    , page : Page
    }


type Page
    = NotFound
    | Loading
    | Login Page.Login.Model
    | Setup Page.Setup.Model
    | Dashboard Page.Dashboard.Model
    | Settings Page.Settings.Model





-- INIT


init : Flags -> Url.Url -> navigationKey -> ( Model navigationKey, Effect.Effect Msg )
init flags url navKey =
    let
        route =
            Route.fromUrl url

        shared =
            Shared.init navKey
                |> (\s -> { s | currentRoute = route })

        initialModel =
            { shared = shared
            , page = Loading
            }
    in
    case flags.token of
        Just token ->
            -- We have a token, verify it
            ( initialModel
            , Effect.VerifyToken token GotTokenVerification
            )

        Nothing ->
            -- No token, check server status
            ( initialModel
            , Effect.CheckServerStatus GotServerStatus
            )


initForTesting : Flags -> Url.Url -> () -> ( Model (), Effect.Effect Msg )
initForTesting flags url () =
    -- Test version - ProgramTest.createApplication expects init to take () instead of Nav.Key
    -- We need to provide a Nav.Key but it's opaque. Based on NavigationKeyExample,
    -- we can use a workaround: create a test model with a dummy key.
    -- Actually, ProgramTest should handle this, but since Nav.Key is opaque,
    -- we'll use a workaround where we create the model directly without calling init.
    let
        route =
            Route.fromUrl url

        -- Create a test model with a dummy Nav.Key
        -- Since Nav.Key is opaque and ProgramTest handles navigation,
        -- we'll use a workaround: create the shared model without a real key
        -- For now, we'll need to modify Shared.init to handle this case
        -- Actually, the simplest solution is to use a test-specific Shared.init
        -- that doesn't require Nav.Key, or use a workaround
        -- For testing, we need a Nav.Key but it's opaque.
        -- ProgramTest handles navigation internally via SimulatedEffect.Navigation,
        -- so the navKey won't be used for actual navigation in tests.
        -- We'll create a test key using a workaround: since we can't create Nav.Key directly,
        -- we'll use a helper that creates one from Browser.
        -- Actually, the simplest workaround is to create a minimal Browser.application
        -- just to get a Nav.Key, but that's complex.
        -- For now, let's use a workaround: create the key from the main program.
        -- Actually, since ProgramTest handles navigation, we can use a dummy approach.
        -- The key insight: in tests, navKey is only used for type checking, not actual navigation.
        -- So we can use a workaround where we get the key from somewhere.
        -- Let's try: use a test helper that provides a key.
        -- For now, using a workaround with a test-specific key creation.
        -- In tests, navigationKey is () and navigation is handled by ProgramTest via withSimulatedEffects
        shared =
            Shared.init ()
                |> (\s -> { s | currentRoute = route })

        initialModel =
            { shared = shared
            , page = Loading
            }
    in
    case flags.token of
        Just token ->
            ( initialModel
            , Effect.VerifyToken token GotTokenVerification
            )

        Nothing ->
            ( initialModel
            , Effect.CheckServerStatus GotServerStatus
            )



-- UPDATE


type Msg
    = UrlChanged Url.Url
    | LinkClicked Browser.UrlRequest
    | SharedMsg Shared.Msg
    | LoginMsg Page.Login.Msg
    | SetupMsg Page.Setup.Msg
    | DashboardMsg Page.Dashboard.Msg
    | SettingsMsg Page.Settings.Msg
    | GotServerStatus (Result String ServerStatus)
    | GotTokenVerification (Result String TokenVerificationResponse)
    | RefreshTokenReceived (Maybe String)
    | GotTokenRefresh (Result String TokenPair)
    | GotGitHubPollingStarted (Result String ())
    | SSEEvent Decode.Value
    | SSEConnectionStateChanged String


update : Msg -> Model navigationKey -> ( Model navigationKey, Effect.Effect Msg )
update msg model =
    case msg of
        UrlChanged url ->
            let
                route =
                    Route.fromUrl url

                newShared =
                    Shared.update (Shared.SetRoute route) model.shared
            in
            case route of
                Just Route.Login ->
                    let
                        ( pageModel, effect ) =
                            Page.Login.init newShared
                    in
                    ( { model
                        | shared = newShared
                        , page = Login pageModel
                      }
                    , Effect.map LoginMsg effect
                    )

                Just Route.Setup ->
                    let
                        ( pageModel, effect ) =
                            Page.Setup.init newShared
                    in
                    ( { model
                        | shared = newShared
                        , page = Setup pageModel
                      }
                    , Effect.map SetupMsg effect
                    )

                Just Route.Dashboard ->
                    case model.page of
                        Dashboard _ ->
                            -- Already on dashboard, just update route
                            ( { model | shared = newShared }
                            , Effect.none
                            )

                        _ ->
                            -- Initialize dashboard
                            let
                                ( pageModel, effect ) =
                                    Page.Dashboard.init newShared
                            in
                            ( { model
                                | shared = newShared
                                , page = Dashboard pageModel
                              }
                            , Effect.map DashboardMsg effect
                            )

                Just (Route.AppDetail appName) ->
                    case model.page of
                        Dashboard dashModel ->
                            -- Stay on dashboard, it will handle the app detail view
                            ( { model | shared = newShared }
                            , Effect.none
                            )

                        _ ->
                            -- Not on dashboard, redirect
                            ( { model | shared = newShared }
                            , Effect.none
                            )

                Just Route.Settings ->
                    let
                        ( pageModel, effect ) =
                            Page.Settings.init newShared
                    in
                    ( { model
                        | shared = newShared
                        , page = Settings pageModel
                      }
                    , Effect.map SettingsMsg effect
                    )

                Nothing ->
                    ( { model
                        | shared = newShared
                        , page = NotFound
                      }
                    , Effect.none
                    )

        LinkClicked urlRequest ->
            case urlRequest of
                Browser.Internal url ->
                    ( model
                    , Effect.PushUrl (Url.toString url)
                    )

                Browser.External href ->
                    ( model
                    , Effect.Load href
                    )

        SharedMsg sharedMsg ->
            ( { model | shared = Shared.update sharedMsg model.shared }
            , Effect.none
            )

        LoginMsg loginMsg ->
            case model.page of
                Login pageModel ->
                    let
                        ( newPageModel, effect ) =
                            Page.Login.update model.shared loginMsg pageModel

                        ( newModel, cmd ) =
                            handleLoginEffect model newPageModel effect
                    in
                    ( newModel, cmd )

                _ ->
                    ( model, Effect.none )

        SetupMsg setupMsg ->
            case model.page of
                Setup pageModel ->
                    let
                        ( newPageModel, effect ) =
                            Page.Setup.update model.shared setupMsg pageModel

                        ( newModel, cmd ) =
                            handleSetupEffect model newPageModel effect
                    in
                    ( newModel, cmd )

                _ ->
                    ( model, Effect.none )

        DashboardMsg dashboardMsg ->
            case model.page of
                Dashboard pageModel ->
                    let
                        ( newPageModel, effect ) =
                            Page.Dashboard.update model.shared dashboardMsg pageModel

                        ( newModel, cmd ) =
                            handleDashboardEffect model newPageModel effect
                    in
                    ( newModel, cmd )

                _ ->
                    ( model, Effect.none )

        SettingsMsg settingsMsg ->
            case model.page of
                Settings pageModel ->
                    let
                        ( newPageModel, effect ) =
                            Page.Settings.update model.shared settingsMsg pageModel
                    in
                    ( { model | page = Settings newPageModel }
                    , Effect.map SettingsMsg effect
                    )

                _ ->
                    ( model, Effect.none )

        GotServerStatus result ->
            case result of
                Ok status ->
                    let
                        newShared =
                            Shared.update (Shared.SetServerVersion status.version) model.shared
                    in
                    if status.initialized then
                        let
                            ( pageModel, effect ) =
                                Page.Login.init newShared
                        in
                        ( { model
                            | shared = newShared
                            , page = Login pageModel
                          }
                        , Effect.batch
                            [ Effect.PushUrl "/login"
                            , Effect.map LoginMsg effect
                            ]
                        )

                    else
                        let
                            ( pageModel, effect ) =
                                Page.Setup.init newShared
                        in
                        ( { model
                            | shared = newShared
                            , page = Setup pageModel
                          }
                        , Effect.batch
                            [ Effect.PushUrl "/setup"
                            , Effect.map SetupMsg effect
                            ]
                        )

                Err _ ->
                    let
                        ( pageModel, effect ) =
                            Page.Login.init model.shared
                    in
                    ( { model | page = Login pageModel }
                    , Effect.batch
                        [ Effect.PushUrl "/login"
                        , Effect.map LoginMsg effect
                        ]
                    )

        GotTokenVerification result ->
            case result of
                Ok response ->
                    -- Token is valid, update shared and get refresh token
                    let
                        newShared =
                            model.shared
                                |> Shared.update (Shared.SetUser response.user)
                                |> Shared.update (Shared.SetTokens response.token "")
                    in
                    ( { model | shared = newShared }
                    , Effect.GetRefreshToken
                    )

                Err _ ->
                    -- Token invalid, try to refresh if we have a refresh token
                    ( model
                    , Effect.GetRefreshToken
                    )

        RefreshTokenReceived maybeRefreshToken ->
            case maybeRefreshToken of
                Just refreshToken ->
                    case ( model.shared.token, model.shared.user ) of
                        ( Just token, Just user ) ->
                            -- We have both token and user, set up dashboard and connect SSE
                            let
                                newShared =
                                    Shared.update (Shared.SetTokens token refreshToken) model.shared

                                ( pageModel, effect ) =
                                    Page.Dashboard.init newShared
                            in
                            ( { model
                                | shared = newShared
                                , page = Dashboard pageModel
                              }
                            , Effect.batch
                                [ Effect.PushUrl "/dashboard"
                                , Effect.SaveRefreshToken refreshToken
                                , Effect.ConnectSSE { token = token, filters = Nothing }
                                , Effect.map DashboardMsg effect
                                ]
                            )

                        _ ->
                            -- No token/user, try to refresh access token
                            ( model
                            , Effect.RefreshAccessToken refreshToken GotTokenRefresh
                            )

                Nothing ->
                    -- No refresh token, clear and check status
                    ( { model | shared = Shared.update Shared.ClearAuth model.shared }
                    , Effect.batch
                        [ Effect.ClearToken
                        , Effect.DisconnectSSE
                        , Effect.CheckServerStatus GotServerStatus
                        ]
                    )

        GotTokenRefresh result ->
            case result of
                Ok tokenPair ->
                    -- We got new tokens, verify the access token
                    ( model
                    , Effect.batch
                        [ Effect.SaveToken tokenPair.accessToken
                        , Effect.SaveRefreshToken tokenPair.refreshToken
                        , Effect.VerifyToken tokenPair.accessToken GotTokenVerification
                        ]
                    )

                Err _ ->
                    -- Refresh failed, clear tokens and redirect to login
                    let
                        newShared =
                            Shared.update Shared.ClearAuth model.shared

                        ( pageModel, effect ) =
                            Page.Login.init newShared
                    in
                    ( { model
                        | shared = newShared
                        , page = Login pageModel
                      }
                    , Effect.batch
                        [ Effect.PushUrl "/login"
                        , Effect.ClearToken
                        , Effect.DisconnectSSE
                        , Effect.map LoginMsg effect
                        ]
                    )

        SSEEvent value ->
            -- Route SSE message to appropriate page based on message type
            -- For now, forward all to Dashboard if we're on that page
            case model.page of
                Dashboard pageModel ->
                    let
                        ( newPageModel, effect ) =
                            Page.Dashboard.update model.shared (Page.Dashboard.HandleSSEEvent value) pageModel
                    in
                    ( { model | page = Dashboard newPageModel }
                    , Effect.map DashboardMsg effect
                    )

                _ ->
                    -- TODO: Handle SSE events on other pages or update global state
                    ( model, Effect.none )

        GotGitHubPollingStarted _ ->
            -- Polling started, events will come via SSE
            ( model, Effect.none )

        SSEConnectionStateChanged state ->
            let
                newShared =
                    Shared.update (Shared.SetSSEConnectionState state) model.shared
            in
            ( { model | shared = newShared }, Effect.none )



-- HANDLE PAGE EFFECTS


handleLoginEffect : Model navigationKey -> Page.Login.Model -> Effect.Effect Page.Login.Msg -> ( Model navigationKey, Effect.Effect Msg )
handleLoginEffect model pageModel effect =
    -- Check if effect contains auth response that we need to handle at main level
    case getAuthResponseFromEffect effect of
        Just authResponse ->
            -- Update shared with user and tokens
            let
                newShared =
                    model.shared
                        |> Shared.update (Shared.SetUser authResponse.user)
                        |> Shared.update (Shared.SetTokens authResponse.accessToken authResponse.refreshToken)

                ( dashModel, dashEffect ) =
                    Page.Dashboard.init newShared
            in
            ( { model
                | shared = newShared
                , page = Dashboard dashModel
              }
            , Effect.batch
                [ Effect.map LoginMsg effect
                , Effect.map DashboardMsg dashEffect
                ]
            )

        Nothing ->
            ( { model | page = Login pageModel }
            , Effect.map LoginMsg effect
            )


handleSetupEffect : Model navigationKey -> Page.Setup.Model -> Effect.Effect Page.Setup.Msg -> ( Model navigationKey, Effect.Effect Msg )
handleSetupEffect model pageModel effect =
    -- Check if effect contains auth response that we need to handle at main level
    case getAuthResponseFromEffect effect of
        Just authResponse ->
            -- Update shared with user and tokens
            let
                newShared =
                    model.shared
                        |> Shared.update (Shared.SetUser authResponse.user)
                        |> Shared.update (Shared.SetTokens authResponse.accessToken authResponse.refreshToken)

                ( dashModel, dashEffect ) =
                    Page.Dashboard.init newShared
            in
            ( { model
                | shared = newShared
                , page = Dashboard dashModel
              }
            , Effect.batch
                [ Effect.map SetupMsg effect
                , Effect.map DashboardMsg dashEffect
                ]
            )

        Nothing ->
            ( { model | page = Setup pageModel }
            , Effect.map SetupMsg effect
            )


handleDashboardEffect : Model navigationKey -> Page.Dashboard.Model -> Effect.Effect Page.Dashboard.Msg -> ( Model navigationKey, Effect.Effect Msg )
handleDashboardEffect model pageModel effect =
    -- Check if effect clears auth
    if containsClearToken effect then
        let
            newShared =
                Shared.update Shared.ClearAuth model.shared

            ( loginModel, loginEffect ) =
                Page.Login.init newShared
        in
        ( { model
            | shared = newShared
            , page = Login loginModel
          }
        , Effect.batch
            [ Effect.map DashboardMsg effect
            , Effect.map LoginMsg loginEffect
            ]
        )

    else
        -- Check if GitHub status changed
        let
            newShared =
                updateSharedFromEffect model.shared effect
        in
        ( { model
            | shared = newShared
            , page = Dashboard pageModel
          }
        , Effect.map DashboardMsg effect
        )



-- EFFECT HELPERS

{-| Extract AuthResponse from an effect if it contains one
-}
getAuthResponseFromEffect : Effect.Effect msg -> Maybe Effect.AuthResponse
getAuthResponseFromEffect effect =
    -- This is a simplified check - in reality, effects are opaque
    -- We'll handle auth responses in performEffect instead
    Nothing


{-| Check if effect contains ClearToken
-}
containsClearToken : Effect.Effect msg -> Bool
containsClearToken effect =
    -- This is a simplified check - effects are opaque
    -- We'll handle logout in performEffect
    False


{-| Update shared model based on effect side effects
-}
updateSharedFromEffect : Shared.Model navigationKey -> Effect.Effect msg -> Shared.Model navigationKey
updateSharedFromEffect shared effect =
    case effect of
        Effect.UpdateGitHubStatus status ->
            let
                sharedStatus =
                    case status of
                        Effect.GitHubUnknown ->
                            Shared.GitHubUnknown

                        Effect.GitHubNotConnected ->
                            Shared.GitHubNotConnected

                        Effect.GitHubConnected username ->
                            Shared.GitHubConnected username
            in
            Shared.update (Shared.SetGitHubStatus sharedStatus) shared

        Effect.Batch effects ->
            List.foldl (\eff acc -> updateSharedFromEffect acc eff) shared effects

        _ ->
            shared



-- VIEW


view : Model navigationKey -> Browser.Document Msg
view model =
    { title = getPageTitle model.page
    , body =
        [ case model.page of
            NotFound ->
                viewNotFound

            Loading ->
                viewLoading

            Login pageModel ->
                Page.Login.view model.shared pageModel
                    |> Html.map LoginMsg

            Setup pageModel ->
                Page.Setup.view model.shared pageModel
                    |> Html.map SetupMsg

            Dashboard pageModel ->
                Page.Dashboard.view model.shared pageModel
                    |> Html.map DashboardMsg

            Settings pageModel ->
                Page.Settings.view model.shared pageModel
                    |> Html.map SettingsMsg
        ]
    }


getPageTitle : Page -> String
getPageTitle page =
    case page of
        NotFound ->
            "Not Found - Litehouse"

        Loading ->
            "Loading - Litehouse"

        Login _ ->
            "Login - Litehouse"

        Setup _ ->
            "Setup - Litehouse"

        Dashboard _ ->
            "Dashboard - Litehouse"

        Settings _ ->
            "Settings - Litehouse"


viewNotFound : Html Msg
viewNotFound =
    div [ class "min-h-screen bg-litehouse-bg flex items-center justify-center" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft p-10 text-center" ]
            [ p [ class "text-litehouse-muted" ] [ text "Page not found" ]
            ]
        ]


viewLoading : Html Msg
viewLoading =
    div [ class "min-h-screen bg-litehouse-bg flex items-center justify-center" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft p-10 text-center" ]
            [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mx-auto mb-4" ] []
            , p [ class "text-litehouse-muted" ] [ text "Loading..." ]
            ]
        ]



-- SUBSCRIPTIONS


subscriptions : Model navigationKey -> Sub Msg
subscriptions model =
    Sub.batch
        [ Ports.refreshTokenReceived RefreshTokenReceived
        , Ports.sseEvent SSEEvent
        , Ports.sseConnectionState SSEConnectionStateChanged
        ]
