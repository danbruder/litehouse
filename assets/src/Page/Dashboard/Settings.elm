module Page.Dashboard.Settings exposing
    ( Model
    , Msg(..)
    , init
    , update
    , view
    )

{-| Settings page for S3 backup configuration.
-}

import Effect exposing (Effect)
import Html exposing (Html, button, div, h2, h3, input, label, p, text)
import Html.Attributes exposing (class, disabled, for, id, placeholder, required, type_, value)
import Html.Events exposing (onClick, onInput, onSubmit)
import Shared


-- MODEL


type alias Model =
    { s3Config : Maybe Effect.S3ConfigRedacted
    , s3ConfigLoading : Bool
    , s3Form : Effect.S3ConfigForm
    , s3FormError : Maybe String
    , s3FormSaving : Bool
    }


init : Model
init =
    { s3Config = Nothing
    , s3ConfigLoading = True
    , s3Form =
        { accessKeyId = ""
        , secretAccessKey = ""
        , bucket = ""
        , region = ""
        , endpoint = ""
        , pathPrefix = ""
        }
    , s3FormError = Nothing
    , s3FormSaving = False
    }


-- UPDATE


type Msg
    = GotS3Config (Result String (Maybe Effect.S3ConfigRedacted))
    | S3FormFieldChanged String String
    | SubmitS3Config
    | GotS3ConfigSet (Result String String)
    | DeleteS3Config
    | GotS3ConfigDeleted (Result String String)


update : Msg -> Model -> String -> ( Model, Effect Msg )
update msg model token =
    case msg of
        GotS3Config result ->
            case result of
                Ok maybeConfig ->
                    let
                        form =
                            case maybeConfig of
                                Just config ->
                                    { accessKeyId = ""
                                    , secretAccessKey = ""
                                    , bucket = config.bucket
                                    , region = config.region
                                    , endpoint = Maybe.withDefault "" config.endpoint
                                    , pathPrefix = Maybe.withDefault "" config.pathPrefix
                                    }

                                Nothing ->
                                    model.s3Form
                    in
                    ( { model
                        | s3Config = maybeConfig
                        , s3ConfigLoading = False
                        , s3Form = form
                      }
                    , Effect.none
                    )

                Err error ->
                    ( { model
                        | s3ConfigLoading = False
                        , s3FormError = Just error
                      }
                    , Effect.none
                    )

        S3FormFieldChanged fieldName fieldValue ->
            let
                form =
                    model.s3Form

                updatedForm =
                    case fieldName of
                        "access_key_id" ->
                            { form | accessKeyId = fieldValue }

                        "secret_access_key" ->
                            { form | secretAccessKey = fieldValue }

                        "bucket" ->
                            { form | bucket = fieldValue }

                        "region" ->
                            { form | region = fieldValue }

                        "endpoint" ->
                            { form | endpoint = fieldValue }

                        "path_prefix" ->
                            { form | pathPrefix = fieldValue }

                        _ ->
                            form
            in
            ( { model | s3Form = updatedForm }
            , Effect.none
            )

        SubmitS3Config ->
            let
                form =
                    model.s3Form
            in
            ( { model
                | s3FormSaving = True
                , s3FormError = Nothing
              }
            , Effect.SetS3Config token form GotS3ConfigSet
            )

        GotS3ConfigSet result ->
            case result of
                Ok _ ->
                    ( { model
                        | s3FormSaving = False
                        , s3FormError = Nothing
                      }
                    , Effect.FetchS3Config token GotS3Config
                    )

                Err error ->
                    ( { model
                        | s3FormSaving = False
                        , s3FormError = Just error
                      }
                    , Effect.none
                    )

        DeleteS3Config ->
            ( { model
                | s3FormSaving = True
                , s3FormError = Nothing
              }
            , Effect.DeleteS3Config token GotS3ConfigDeleted
            )

        GotS3ConfigDeleted result ->
            case result of
                Ok _ ->
                    ( { model
                        | s3Config = Nothing
                        , s3FormSaving = False
                        , s3FormError = Nothing
                        , s3Form = init.s3Form
                      }
                    , Effect.none
                    )

                Err error ->
                    ( { model
                        | s3FormSaving = False
                        , s3FormError = Just error
                      }
                    , Effect.none
                    )


-- VIEW


view : Shared.Model navigationKey -> Model -> Html Msg
view shared model =
    div [ class "max-w-3xl" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ div [ class "flex justify-between items-center mb-6 pb-4 border-b border-litehouse-border" ]
                [ h2 [ class "text-xl font-semibold text-litehouse-text" ] [ text "Settings" ]
                ]
            , div []
                [ h3 [ class "text-lg font-medium text-litehouse-text mb-4" ] [ text "S3 Backup Configuration" ]
                , p [ class "text-sm text-litehouse-muted mb-6" ]
                    [ text "Configure S3 credentials for Litestream backups. These credentials will be used for all app databases and the main Litehouse database." ]
                , if model.s3ConfigLoading then
                    div [ class "py-8 text-center" ]
                        [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mx-auto mb-4" ] []
                        , p [ class "text-litehouse-muted" ] [ text "Loading S3 configuration..." ]
                        ]

                  else
                    viewS3ConfigForm model
                ]
            ]
        ]


viewS3ConfigForm : Model -> Html Msg
viewS3ConfigForm model =
    let
        form =
            model.s3Form

        hasConfig =
            model.s3Config /= Nothing
    in
    div []
        [ if hasConfig then
            div [ class "mb-4 p-4 bg-litehouse-success/10 border border-litehouse-success/30 rounded-xl" ]
                [ p [ class "text-sm text-litehouse-success font-medium" ] [ text "S3 configuration is active" ]
                , case model.s3Config of
                    Just config ->
                        div [ class "mt-2 text-xs text-litehouse-muted space-y-1" ]
                            [ p [] [ text ("Bucket: " ++ config.bucket) ]
                            , p [] [ text ("Region: " ++ config.region) ]
                            , case config.endpoint of
                                Just endpoint ->
                                    p [] [ text ("Endpoint: " ++ endpoint) ]

                                Nothing ->
                                    text ""
                            ]

                    Nothing ->
                        text ""
                ]

          else
            text ""
        , Html.form [ onSubmit SubmitS3Config ]
            [ div [ class "space-y-4" ]
                [ div []
                    [ label [ for "s3_access_key_id", class "block mb-1 text-sm font-medium text-litehouse-text" ]
                        [ text "Access Key ID" ]
                    , input
                        [ type_ "text"
                        , id "s3_access_key_id"
                        , value form.accessKeyId
                        , onInput (S3FormFieldChanged "access_key_id")
                        , placeholder "AKIAIOSFODNN7EXAMPLE"
                        , required True
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50"
                        ]
                        []
                    ]
                , div []
                    [ label [ for "s3_secret_access_key", class "block mb-1 text-sm font-medium text-litehouse-text" ]
                        [ text "Secret Access Key" ]
                    , input
                        [ type_ "password"
                        , id "s3_secret_access_key"
                        , value form.secretAccessKey
                        , onInput (S3FormFieldChanged "secret_access_key")
                        , placeholder "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
                        , required True
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50"
                        ]
                        []
                    ]
                , div []
                    [ label [ for "s3_bucket", class "block mb-1 text-sm font-medium text-litehouse-text" ]
                        [ text "Bucket" ]
                    , input
                        [ type_ "text"
                        , id "s3_bucket"
                        , value form.bucket
                        , onInput (S3FormFieldChanged "bucket")
                        , placeholder "my-backup-bucket"
                        , required True
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50"
                        ]
                        []
                    ]
                , div []
                    [ label [ for "s3_region", class "block mb-1 text-sm font-medium text-litehouse-text" ]
                        [ text "Region" ]
                    , input
                        [ type_ "text"
                        , id "s3_region"
                        , value form.region
                        , onInput (S3FormFieldChanged "region")
                        , placeholder "us-east-1"
                        , required True
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50"
                        ]
                        []
                    ]
                , div []
                    [ label [ for "s3_endpoint", class "block mb-1 text-sm font-medium text-litehouse-text" ]
                        [ text "Endpoint (Optional)" ]
                    , input
                        [ type_ "text"
                        , id "s3_endpoint"
                        , value form.endpoint
                        , onInput (S3FormFieldChanged "endpoint")
                        , placeholder "https://s3.example.com (for S3-compatible services)"
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50"
                        ]
                        []
                    , p [ class "mt-1 text-xs text-litehouse-muted" ] [ text "Leave empty for standard AWS S3" ]
                    ]
                , div []
                    [ label [ for "s3_path_prefix", class "block mb-1 text-sm font-medium text-litehouse-text" ]
                        [ text "Path Prefix (Optional)" ]
                    , input
                        [ type_ "text"
                        , id "s3_path_prefix"
                        , value form.pathPrefix
                        , onInput (S3FormFieldChanged "path_prefix")
                        , placeholder "litehouse"
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50"
                        ]
                        []
                    , p [ class "mt-1 text-xs text-litehouse-muted" ] [ text "Prefix for all backup paths (default: litehouse)" ]
                    ]
                ]
            , if model.s3FormError /= Nothing then
                div [ class "mt-4 p-3 bg-litehouse-error/10 border border-litehouse-error/30 rounded-xl" ]
                    [ p [ class "text-sm text-litehouse-error" ]
                        [ text (Maybe.withDefault "" model.s3FormError) ]
                    ]

              else
                text ""
            , div [ class "mt-6 flex gap-3" ]
                [ button
                    [ type_ "submit"
                    , disabled model.s3FormSaving
                    , class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                    ]
                    [ if model.s3FormSaving then
                        text "Saving..."

                      else if hasConfig then
                        text "Update Configuration"

                      else
                        text "Save Configuration"
                    ]
                , if hasConfig then
                    button
                        [ type_ "button"
                        , onClick DeleteS3Config
                        , disabled model.s3FormSaving
                        , class "px-4 py-2 border border-litehouse-border text-litehouse-error hover:bg-litehouse-error/10 rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                        ]
                        [ text "Delete Configuration" ]

                  else
                    text ""
                ]
            ]
        ]
