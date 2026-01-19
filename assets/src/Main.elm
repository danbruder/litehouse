port module Main exposing (main)

import Browser
import Html exposing (..)
import Html.Attributes exposing (..)
import Html.Events exposing (onClick, onInput, onSubmit)
import Http
import Json.Decode as Decode
import Json.Encode as Encode


-- PORTS


port saveToken : String -> Cmd msg


port clearToken : () -> Cmd msg



-- MAIN


main : Program Flags Model Msg
main =
    Browser.element
        { init = init
        , update = update
        , view = view
        , subscriptions = \_ -> Sub.none
        }



-- FLAGS


type alias Flags =
    { token : Maybe String
    }



-- MODEL


type Page
    = Loading
    | Setup SetupForm
    | Login LoginForm
    | Dashboard UserInfo


type alias SetupForm =
    { email : String
    , password : String
    , confirmPassword : String
    , fullName : String
    , organizationName : String
    , error : Maybe String
    , submitting : Bool
    }


type alias LoginForm =
    { email : String
    , password : String
    , error : Maybe String
    , submitting : Bool
    }


type alias UserInfo =
    { email : String
    , fullName : String
    }


type alias Model =
    { page : Page
    , serverVersion : String
    }


emptySetupForm : SetupForm
emptySetupForm =
    { email = ""
    , password = ""
    , confirmPassword = ""
    , fullName = ""
    , organizationName = ""
    , error = Nothing
    , submitting = False
    }


emptyLoginForm : LoginForm
emptyLoginForm =
    { email = ""
    , password = ""
    , error = Nothing
    , submitting = False
    }


init : Flags -> ( Model, Cmd Msg )
init flags =
    case flags.token of
        Just token ->
            -- We have a token, verify it
            ( { page = Loading, serverVersion = "" }
            , verifyToken token
            )

        Nothing ->
            -- No token, check server status
            ( { page = Loading, serverVersion = "" }
            , checkServerStatus
            )



-- UPDATE


type Msg
    = GotServerStatus (Result Http.Error ServerStatus)
    | GotTokenVerification (Result Http.Error UserInfo)
    | GotLoginResponse (Result Http.Error AuthResponse)
    | GotRegisterResponse (Result Http.Error AuthResponse)
      -- Setup form
    | SetupEmailChanged String
    | SetupPasswordChanged String
    | SetupConfirmPasswordChanged String
    | SetupFullNameChanged String
    | SetupOrganizationNameChanged String
    | SubmitSetup
      -- Login form
    | LoginEmailChanged String
    | LoginPasswordChanged String
    | SubmitLogin
      -- Dashboard
    | Logout


type alias ServerStatus =
    { initialized : Bool
    , version : String
    }


type alias AuthResponse =
    { accessToken : String
    , user : UserInfo
    }


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        GotServerStatus result ->
            case result of
                Ok status ->
                    if status.initialized then
                        ( { model
                            | page = Login emptyLoginForm
                            , serverVersion = status.version
                          }
                        , Cmd.none
                        )

                    else
                        ( { model
                            | page = Setup emptySetupForm
                            , serverVersion = status.version
                          }
                        , Cmd.none
                        )

                Err _ ->
                    ( { model | page = Login { emptyLoginForm | error = Just "Failed to connect to server" } }
                    , Cmd.none
                    )

        GotTokenVerification result ->
            case result of
                Ok userInfo ->
                    ( { model | page = Dashboard userInfo }
                    , Cmd.none
                    )

                Err _ ->
                    -- Token invalid, clear it and check status
                    ( { model | page = Loading }
                    , Cmd.batch [ clearToken (), checkServerStatus ]
                    )

        GotLoginResponse result ->
            case model.page of
                Login form ->
                    case result of
                        Ok response ->
                            ( { model | page = Dashboard response.user }
                            , saveToken response.accessToken
                            )

                        Err err ->
                            ( { model | page = Login { form | error = Just (httpErrorToString err), submitting = False } }
                            , Cmd.none
                            )

                _ ->
                    ( model, Cmd.none )

        GotRegisterResponse result ->
            case model.page of
                Setup form ->
                    case result of
                        Ok response ->
                            ( { model | page = Dashboard response.user }
                            , saveToken response.accessToken
                            )

                        Err err ->
                            ( { model | page = Setup { form | error = Just (httpErrorToString err), submitting = False } }
                            , Cmd.none
                            )

                _ ->
                    ( model, Cmd.none )

        -- Setup form messages
        SetupEmailChanged email ->
            case model.page of
                Setup form ->
                    ( { model | page = Setup { form | email = email } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SetupPasswordChanged password ->
            case model.page of
                Setup form ->
                    ( { model | page = Setup { form | password = password } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SetupConfirmPasswordChanged confirmPassword ->
            case model.page of
                Setup form ->
                    ( { model | page = Setup { form | confirmPassword = confirmPassword } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SetupFullNameChanged fullName ->
            case model.page of
                Setup form ->
                    ( { model | page = Setup { form | fullName = fullName } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SetupOrganizationNameChanged orgName ->
            case model.page of
                Setup form ->
                    ( { model | page = Setup { form | organizationName = orgName } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SubmitSetup ->
            case model.page of
                Setup form ->
                    if form.password /= form.confirmPassword then
                        ( { model | page = Setup { form | error = Just "Passwords do not match" } }
                        , Cmd.none
                        )

                    else if String.length form.password < 8 then
                        ( { model | page = Setup { form | error = Just "Password must be at least 8 characters" } }
                        , Cmd.none
                        )

                    else
                        ( { model | page = Setup { form | submitting = True, error = Nothing } }
                        , submitRegister form
                        )

                _ ->
                    ( model, Cmd.none )

        -- Login form messages
        LoginEmailChanged email ->
            case model.page of
                Login form ->
                    ( { model | page = Login { form | email = email } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        LoginPasswordChanged password ->
            case model.page of
                Login form ->
                    ( { model | page = Login { form | password = password } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SubmitLogin ->
            case model.page of
                Login form ->
                    ( { model | page = Login { form | submitting = True, error = Nothing } }
                    , submitLogin form
                    )

                _ ->
                    ( model, Cmd.none )

        Logout ->
            ( { model | page = Login emptyLoginForm }
            , clearToken ()
            )



-- HTTP


checkServerStatus : Cmd Msg
checkServerStatus =
    Http.get
        { url = "/api/auth/status"
        , expect = Http.expectJson GotServerStatus serverStatusDecoder
        }


verifyToken : String -> Cmd Msg
verifyToken token =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/auth/me"
        , body = Http.emptyBody
        , expect = Http.expectJson GotTokenVerification userInfoDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


submitLogin : LoginForm -> Cmd Msg
submitLogin form =
    Http.post
        { url = "/api/auth/login"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "email", Encode.string form.email )
                    , ( "password", Encode.string form.password )
                    ]
                )
        , expect = Http.expectJson GotLoginResponse authResponseDecoder
        }


submitRegister : SetupForm -> Cmd Msg
submitRegister form =
    Http.post
        { url = "/api/auth/register"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "email", Encode.string form.email )
                    , ( "password", Encode.string form.password )
                    , ( "full_name", Encode.string form.fullName )
                    , ( "organization_name", Encode.string form.organizationName )
                    ]
                )
        , expect = Http.expectJson GotRegisterResponse authResponseDecoder
        }



-- DECODERS


serverStatusDecoder : Decode.Decoder ServerStatus
serverStatusDecoder =
    Decode.map2 ServerStatus
        (Decode.field "initialized" Decode.bool)
        (Decode.field "version" Decode.string)


userInfoDecoder : Decode.Decoder UserInfo
userInfoDecoder =
    Decode.map2 UserInfo
        (Decode.at [ "user", "email" ] Decode.string)
        (Decode.at [ "user", "full_name" ] Decode.string)


authResponseDecoder : Decode.Decoder AuthResponse
authResponseDecoder =
    Decode.map2 AuthResponse
        (Decode.at [ "tokens", "access_token" ] Decode.string)
        (Decode.field "user" userDecoder)


userDecoder : Decode.Decoder UserInfo
userDecoder =
    Decode.map2 UserInfo
        (Decode.field "email" Decode.string)
        (Decode.field "full_name" Decode.string)



-- HELPERS


httpErrorToString : Http.Error -> String
httpErrorToString error =
    case error of
        Http.BadUrl url ->
            "Invalid URL: " ++ url

        Http.Timeout ->
            "Request timed out"

        Http.NetworkError ->
            "Network error - please check your connection"

        Http.BadStatus status ->
            case status of
                401 ->
                    "Invalid email or password"

                409 ->
                    "An account with this email already exists"

                _ ->
                    "Server error (status " ++ String.fromInt status ++ ")"

        Http.BadBody message ->
            "Error parsing response: " ++ message



-- VIEW


view : Model -> Html Msg
view model =
    case model.page of
        Loading ->
            viewLoading

        Setup form ->
            viewSetup form model.serverVersion

        Login form ->
            viewLogin form model.serverVersion

        Dashboard userInfo ->
            viewDashboard userInfo model.serverVersion


viewLoading : Html Msg
viewLoading =
    div [ class "auth-container" ]
        [ div [ class "auth-card" ]
            [ div [ class "spinner" ] []
            , p [ class "loading-text" ] [ text "Loading..." ]
            ]
        ]


viewSetup : SetupForm -> String -> Html Msg
viewSetup form version =
    div [ class "auth-container" ]
        [ div [ class "auth-card" ]
            [ h1 [] [ text "Welcome to Litehouse" ]
            , p [ class "subtitle" ] [ text "Create your admin account to get started" ]
            , Html.form [ onSubmit SubmitSetup ]
                [ viewError form.error
                , div [ class "form-group" ]
                    [ label [ for "fullName" ] [ text "Full Name" ]
                    , input
                        [ type_ "text"
                        , id "fullName"
                        , value form.fullName
                        , onInput SetupFullNameChanged
                        , placeholder "John Doe"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , div [ class "form-group" ]
                    [ label [ for "email" ] [ text "Email" ]
                    , input
                        [ type_ "email"
                        , id "email"
                        , value form.email
                        , onInput SetupEmailChanged
                        , placeholder "admin@example.com"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , div [ class "form-group" ]
                    [ label [ for "orgName" ] [ text "Organization Name" ]
                    , input
                        [ type_ "text"
                        , id "orgName"
                        , value form.organizationName
                        , onInput SetupOrganizationNameChanged
                        , placeholder "My Organization"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , div [ class "form-group" ]
                    [ label [ for "password" ] [ text "Password" ]
                    , input
                        [ type_ "password"
                        , id "password"
                        , value form.password
                        , onInput SetupPasswordChanged
                        , placeholder "At least 8 characters"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , div [ class "form-group" ]
                    [ label [ for "confirmPassword" ] [ text "Confirm Password" ]
                    , input
                        [ type_ "password"
                        , id "confirmPassword"
                        , value form.confirmPassword
                        , onInput SetupConfirmPasswordChanged
                        , placeholder "Re-enter your password"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , button
                    [ type_ "submit"
                    , class "btn-primary"
                    , disabled form.submitting
                    ]
                    [ text
                        (if form.submitting then
                            "Creating Account..."

                         else
                            "Create Account"
                        )
                    ]
                ]
            , viewVersion version
            ]
        ]


viewLogin : LoginForm -> String -> Html Msg
viewLogin form version =
    div [ class "auth-container" ]
        [ div [ class "auth-card" ]
            [ h1 [] [ text "Litehouse" ]
            , p [ class "subtitle" ] [ text "Sign in to your account" ]
            , Html.form [ onSubmit SubmitLogin ]
                [ viewError form.error
                , div [ class "form-group" ]
                    [ label [ for "email" ] [ text "Email" ]
                    , input
                        [ type_ "email"
                        , id "email"
                        , value form.email
                        , onInput LoginEmailChanged
                        , placeholder "admin@example.com"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , div [ class "form-group" ]
                    [ label [ for "password" ] [ text "Password" ]
                    , input
                        [ type_ "password"
                        , id "password"
                        , value form.password
                        , onInput LoginPasswordChanged
                        , placeholder "Your password"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , button
                    [ type_ "submit"
                    , class "btn-primary"
                    , disabled form.submitting
                    ]
                    [ text
                        (if form.submitting then
                            "Signing in..."

                         else
                            "Sign In"
                        )
                    ]
                ]
            , viewVersion version
            ]
        ]


viewDashboard : UserInfo -> String -> Html Msg
viewDashboard userInfo version =
    div [ class "dashboard" ]
        [ header [ class "dashboard-header" ]
            [ div [ class "header-brand" ]
                [ h1 [] [ text "Litehouse" ]
                ]
            , div [ class "header-user" ]
                [ span [ class "user-name" ] [ text userInfo.fullName ]
                , button [ class "btn-secondary", onClick Logout ] [ text "Logout" ]
                ]
            ]
        , main_ [ class "dashboard-content" ]
            [ div [ class "welcome-card" ]
                [ h2 [] [ text ("Welcome, " ++ userInfo.fullName ++ "!") ]
                , p [] [ text "Your Litehouse server is ready." ]
                , p [ class "user-email" ] [ text userInfo.email ]
                ]
            ]
        , footer [ class "dashboard-footer" ]
            [ viewVersion version
            ]
        ]


viewError : Maybe String -> Html Msg
viewError maybeError =
    case maybeError of
        Just error ->
            div [ class "error-message" ] [ text error ]

        Nothing ->
            text ""


viewVersion : String -> Html Msg
viewVersion version =
    if String.isEmpty version then
        text ""

    else
        p [ class "version" ] [ text ("v" ++ version) ]
